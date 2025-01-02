/// Core imports
use core::fmt::Write;
use embassy_net::{tcp::TcpSocket, Stack};
use embassy_time::{Duration, Timer};
// use embedded_io_async::Write as EmbeddedIoWrite;
use esp_println::println;
use esp_wifi::wifi::{WifiDevice, WifiStaDevice};

/// Crate imports
use crate::close_socket;
use crate::get_request;
use crate::send_response_buffer;
use crate::write_response_headers;
use crate::write_response_status;
use crate::Coordinate;
use crate::CoordinateList;
use crate::GridData;
use crate::BACKEND_ENDPOINT;
use esp32_drawer::buffer::ResponseBuffer;

#[embassy_executor::task]
// async fn backend_loop(stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>) {
pub async fn task_loop(stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>) {
    println!("Starting backend_loop\r\n");
    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];
    let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
    socket.set_timeout(Some(embassy_time::Duration::from_secs(10)));

    let mut grid_data = GridData {
        data: [[0; 50]; 50],
    };

    loop {
        let r = socket.accept(BACKEND_ENDPOINT).await;

        if let Err(e) = r {
            // close the socket if it is in at invalid state
            println!("connect error: {:?}\r\n", e);
            close_socket(&mut socket).await;
            continue;
        }

        let mut buffer = [0u8; 512];
        let mut response_buffer = ResponseBuffer::<1024>::new();
        if let Err(e) = get_request(&mut socket, &mut buffer).await {
            println!("backend_loop: {:?}", e);
            continue;
        }

        let request_str = match core::str::from_utf8(&buffer) {
            Ok(result) => result,
            Err(e) => {
                println!("backend_loop: {:?}", e);
                continue;
            }
        };

        let mut lines = request_str.split("\r\n");
        let first_line = lines.next().unwrap_or("");
        let mut parts = first_line.split(' ');
        let method = parts.next().unwrap_or("");
        let path = parts.next().unwrap_or("");

        println!("backend_loop: {:?} {:?}\r\n", method, path);

        match method {
            "GET" => match path {
                "/data" => {
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
            "POST" => match path {
                "/data" => {
                    for line in lines.by_ref() {
                        println!("{:?}", line);
                        if line.is_empty() {
                            break;
                        }
                    }
                    let data = lines.next().unwrap_or("").trim_matches(char::from(0));
                    println!("{:?}", data);
                    match serde_json_core::from_str::<[Option<Coordinate>; 10]>(data) {
                        Ok(result) => {
                            let coord_list = result.0;
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
                "/clear" => {
                    grid_data.data = [[0; 50]; 50];
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
