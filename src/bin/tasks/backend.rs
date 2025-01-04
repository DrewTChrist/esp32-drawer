/// Core imports
use core::fmt::Write;
use embassy_net::{tcp::TcpSocket, Stack};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};
// use embedded_io_async::Write as EmbeddedIoWrite;
use esp_println::println;
use esp_wifi::wifi::{WifiDevice, WifiStaDevice};
use serde::{Deserialize, Serialize, Serializer};

/// Crate imports
use crate::BACKEND_ENDPOINT;
use esp32_drawer::buffer::{RequestBuffer, ResponseBuffer};
use esp32_drawer::close_socket;
use esp32_drawer::get_request;
use esp32_drawer::send_response_buffer;
use esp32_drawer::write_response_headers;
use esp32_drawer::write_response_status;
use esp32_drawer::Coordinates;
use esp32_drawer::Request;
use esp32_drawer::ScreenSignal;

struct GridData {
    data: [[u8; 80]; 64],
}

type Coordinate = (usize, usize);

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct CoordinateList {
    #[serde(serialize_with = "ignore_none")]
    coords: serde_big_array::Array<Option<Coordinate>, 256>,
}

impl CoordinateList {
    fn new() -> Self {
        Self {
            coords: serde_big_array::Array([None; 256]),
        }
    }
}

fn ignore_none<S>(array: &[Option<Coordinate>; 256], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let filtered_array = array.iter().filter_map(|x| x.as_ref());
    serializer.collect_seq(filtered_array)
}

#[embassy_executor::task]
pub async fn task_loop(
    stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>,
    signal: &'static Signal<NoopRawMutex, ScreenSignal>,
) {
    println!("Starting backend_loop\r\n");
    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];
    let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
    socket.set_timeout(Some(embassy_time::Duration::from_secs(10)));

    let mut grid_data = GridData {
        data: [[0; 80]; 64],
    };

    loop {
        let r = socket.accept(BACKEND_ENDPOINT).await;

        if let Err(e) = r {
            // close the socket if it is in at invalid state
            println!("connect error: {:?}\r\n", e);
            close_socket(&mut socket).await;
            continue;
        }

        let mut request_buffer = RequestBuffer::<512>::new();
        let mut response_buffer = ResponseBuffer::<1024>::new();
        if let Err(e) = get_request(&mut socket, &mut request_buffer).await {
            println!("backend_loop: {:?}", e);
            continue;
        }

        let mut request: Request<512> = Request::new();
        request.set_request_buffer(&request_buffer);
        request.parse_request();

        // println!("backend_loop: {:?} {:?}\r\n", request.method, request.path);

        match request.method {
            Some("GET") => match request.path {
                Some("/data") => {
                    let mut coordinates = CoordinateList::new();
                    let mut position = 0;
                    for (r_idx, row) in grid_data.data.iter().enumerate() {
                        for (c_idx, col) in row.iter().enumerate() {
                            if *col != 0 && position < coordinates.coords.len() {
                                coordinates.coords[position] = Some((r_idx, c_idx));
                                position += 1;
                            }
                        }
                    }
                    let mut buffer = [0; 2048];
                    match serde_json_core::to_slice(&coordinates, &mut buffer[..]) {
                        Ok(len) => {
                            write_response_status(&mut response_buffer, 200);
                            let _ =
                                write!(&mut response_buffer, "Content-Type: application/json\r\n");
                            let _ = write!(&mut response_buffer, "Content-Length: {}\r\n", len);
                            write_response_headers(&mut response_buffer);
                            let _ = response_buffer.write(&buffer[..len]);
                            // println!("Bytes converted: {:?}\r\n", len);
                        }
                        Err(e) => {
                            println!("{:?}", e);
                            write_response_status(&mut response_buffer, 500);
                            write_response_headers(&mut response_buffer);
                        }
                    }
                }
                _ => {
                    write_response_status(&mut response_buffer, 404);
                    write_response_headers(&mut response_buffer);
                }
            },
            Some("POST") => match request.path {
                Some("/data") => {
                    match serde_json_core::from_str::<Coordinates>(request.data.unwrap()) {
                        Ok(result) => {
                            let coord_list = result.0;
                            signal.signal(ScreenSignal::Coordinate(coord_list));
                            for coordinate in coord_list.iter().flatten() {
                                grid_data.data[coordinate.0][coordinate.1] = 1;
                            }
                        }
                        Err(e) => {
                            println!("Error converting coordinates: {:?}", e);
                        }
                    }
                    write_response_status(&mut response_buffer, 200);
                    write_response_headers(&mut response_buffer);
                }
                Some("/clear") => {
                    grid_data.data = [[0; 80]; 64];
                    signal.signal(ScreenSignal::Clear);
                    write_response_status(&mut response_buffer, 200);
                    write_response_headers(&mut response_buffer);
                }
                _ => {
                    write_response_status(&mut response_buffer, 404);
                    write_response_headers(&mut response_buffer);
                }
            },
            _ => {
                write_response_status(&mut response_buffer, 404);
                write_response_headers(&mut response_buffer);
            }
        }

        send_response_buffer(&mut socket, response_buffer).await;

        let r = socket.flush().await;
        if let Err(e) = r {
            println!("AP flush error: {:?}\r\n", e);
        }
        Timer::after(Duration::from_millis(50)).await;
        socket.close();
        Timer::after(Duration::from_millis(50)).await;
        socket.abort();
    }
}
