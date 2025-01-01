#![no_std]
#![no_main]

/// Core imports
use core::fmt::Write;

/// External imports
use embassy_executor::Spawner;
use embassy_net::{tcp::TcpSocket, IpListenEndpoint, Stack, StackResources};
use embassy_time::{Duration, Timer};
use embedded_io_async::Write as EmbeddedIoWrite;
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::{prelude::*, rng::Rng, timer::timg::TimerGroup};
use esp_println::println;
use esp_wifi::{
    init,
    wifi::{
        ClientConfiguration, Configuration, WifiController, WifiDevice, WifiEvent, WifiStaDevice,
        WifiState,
    },
    EspWifiController,
};

use serde::{Deserialize, Serialize, Serializer};

/// Crate imports
use esp32_drawer::buffer::ResponseBuffer;

// When you are okay with using a nightly compiler it's better to use https://docs.rs/static_cell/2.1.0/static_cell/macro.make_static.html
macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");

const WEB_ENDPOINT: IpListenEndpoint = IpListenEndpoint {
    addr: None,
    port: 8080,
};

const BACKEND_ENDPOINT: IpListenEndpoint = IpListenEndpoint {
    addr: None,
    port: 5000,
};

struct GridData {
    data: [[u8; 50]; 50],
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

#[derive(Debug)]
enum WebServeFile<'a> {
    File(&'a [u8], &'a str),
    NotFound,
}

const INDEX: WebServeFile<'static> =
    WebServeFile::File(include_bytes!("../index.html"), "text/html");
const CSS: WebServeFile<'static> =
    WebServeFile::File(include_bytes!("../css/style.css"), "text/css");

fn path_to_file(path: &str) -> WebServeFile {
    match path {
        "/" => INDEX,
        "/css/style.css" => CSS,
        _ => WebServeFile::NotFound,
    }
}

async fn _print_request<'a, const S: usize>(socket: &mut TcpSocket<'a>) {
    let mut buffer = [0; S];
    let mut pos = 0;
    loop {
        match socket.read(&mut buffer).await {
            Ok(0) => {
                println!("AP read EOF\r\n");
                break;
            }
            Ok(len) => match core::str::from_utf8(&buffer[..(pos + len)]) {
                Ok(to_print) => {
                    if to_print.contains("\r\n\r\n") {
                        println!("{}\r\n", to_print);
                        println!();
                        break;
                    }
                    pos += len;
                }
                Err(e) => {
                    println!("AP read error: {:?}\r\n", e);
                }
            },
            Err(e) => {
                println!("AP read error: {:?}\r\n", e);
                break;
            }
        };
    }
}

async fn get_request<'a, 'b>(
    socket: &mut TcpSocket<'a>,
    buffer: &'b mut [u8],
) -> Result<(), embassy_net::tcp::Error> {
    let mut pos = 0;
    loop {
        match socket.read(buffer).await {
            Ok(0) => {
                println!("AP read EOF\r\n");
                return Err(embassy_net::tcp::Error::ConnectionReset);
            }
            Ok(len) => match core::str::from_utf8(&buffer[..(pos + len)]) {
                Ok(to_print) => {
                    if to_print.contains("\r\n\r\n") {
                        break;
                    }
                    pos += len;
                }
                Err(e) => {
                    println!("AP read error: {:?}\r\n", e);
                }
            },
            Err(e) => return Err(e),
        };
    }
    Ok(())
}

async fn _send_response_status<'a>(socket: &mut TcpSocket<'a>, status_code: usize) {
    let mut status: Option<&[u8]> = None;
    match status_code {
        200 => status = Some(b"HTTP/1.1 200 OK\r\n\r\n"),
        500 => status = Some(b"HTTP/1.1 500 Internal Server Error\r\n\r\n"),
        404 => status = Some(b"HTTP/1.1 404 Not Found\r\n\r\n"),
        _ => {}
    }
    if let Some(response) = status {
        if let Err(e) = socket.write_all(response).await {
            println!("AP write error: {:?}\r\n", e);
        }
    }
}

fn write_response_status<const S: usize>(
    response_buffer: &mut ResponseBuffer<S>,
    status_code: usize,
) {
    let mut status: &str = "";
    match status_code {
        200 => status = "HTTP/1.1 200 OK\r\n",
        500 => status = "HTTP/1.1 500 Internal Server Error\r\n",
        404 => status = "HTTP/1.1 404 Not Found\r\n",
        _ => {}
    }
    if let Err(e) = write!(response_buffer, "{}", status) {
        println!("Error writing response status: {:?}", e);
    }
}

fn write_response_headers<const S: usize>(response_buffer: &mut ResponseBuffer<S>) {
    if let Err(e) = write!(response_buffer, "Access-Control-Allow-Origin: *\r\n") {
        println!("Error writing response headers: {:?}", e);
    }
    if let Err(e) = write!(response_buffer, "\r\n") {
        println!("Error writing response headers: {:?}", e);
    }
}

async fn send_response_buffer<'a, const S: usize>(
    socket: &mut TcpSocket<'a>,
    response_buffer: ResponseBuffer<S>,
) {
    if let Err(e) = socket.write_all(response_buffer.buffer()).await {
        println!("AP write error: {:?}\r\n", e);
    }
}

#[esp_hal_embassy::main]
async fn main(spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();
    let mut config = esp_hal::Config::default();
    config.cpu_clock = CpuClock::max();
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(72 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let mut rng = Rng::new(peripherals.RNG);

    let init = &*mk_static!(
        EspWifiController<'static>,
        init(timg0.timer0, rng, peripherals.RADIO_CLK).unwrap()
    );

    let wifi = peripherals.WIFI;
    let (wifi_interface, controller) =
        esp_wifi::wifi::new_with_mode(init, wifi, WifiStaDevice).unwrap();

    let timg1 = TimerGroup::new(peripherals.TIMG1);
    esp_hal_embassy::init(timg1.timer0);

    let config = embassy_net::Config::dhcpv4(Default::default());

    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    // Init network stack
    let stack = &*mk_static!(
        Stack<WifiDevice<'_, WifiStaDevice>>,
        Stack::new(
            wifi_interface,
            config,
            mk_static!(StackResources<4>, StackResources::<4>::new()),
            seed
        )
    );

    spawner.spawn(connection(controller)).ok();
    spawner.spawn(net_task(stack)).ok();

    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    println!("Waiting to get IP address...\r\n");
    loop {
        if let Some(config) = stack.config_v4() {
            println!("Got IP: {}\r\n", config.address);
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    spawner.spawn(web_serve_loop(stack)).ok();
    spawner.spawn(backend_loop(stack)).ok();

    loop {
        Timer::after(Duration::from_millis(500)).await;
    }
}

#[embassy_executor::task]
async fn backend_loop(stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>) {
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
            println!("connect error: {:?}\r\n", e);
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
                        },
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

#[embassy_executor::task]
async fn web_serve_loop(stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>) {
    println!("Starting web_serve_loop\r\n");
    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];
    let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
    socket.set_timeout(Some(embassy_time::Duration::from_secs(10)));

    loop {
        let r = socket.accept(WEB_ENDPOINT).await;

        if let Err(e) = r {
            println!("connect error: {:?}\r\n", e);
            continue;
        }

        let mut buffer = [0u8; 512];
        if let Err(e) = get_request(&mut socket, &mut buffer).await {
            println!("web_serve_loop: {:?}", e);
            continue;
        }

        let request_str = match core::str::from_utf8(&buffer) {
            Ok(result) => result,
            Err(e) => {
                println!("web_serve_loop: {:?}", e);
                continue;
            }
        };

        let mut lines = request_str.split("\r\n");
        let first_line = lines.next().unwrap_or("");
        let mut parts = first_line.split(' ');
        let method = parts.next().unwrap_or("");
        let path = parts.next().unwrap_or("");

        println!("web_serve_loop: {:?} {:?}\r\n", method, path);

        let mut file_bytes = None;

        match method {
            "GET" => match path_to_file(path) {
                WebServeFile::File(contents, content_type) => {
                    write_response_status(&mut response_buffer, 200);
                    let _ = write!(&mut response_buffer, "Content-Type: {}\r\n", content_type);
                    let _ = write!(
                        &mut response_buffer,
                        "Content-Length: {}\r\n",
                        contents.len()
                    );
                    let _ = write!(&mut response_buffer, "\r\n");
                    file_bytes = Some(contents);
                }
                WebServeFile::NotFound => {
                    write_response_status(&mut response_buffer, 404);
                    let _ = write!(&mut response_buffer, "\r\n");
                }
            },
            _ => {
                write_response_status(&mut response_buffer, 404);
                let _ = write!(&mut response_buffer, "\r\n");
            }
        }

        send_response_buffer(&mut socket, response_buffer).await;

        if let Some(bytes) = file_bytes {
            if let Err(e) = socket.write_all(bytes).await {
                println!("web_serve_loop write error: {:?}\r\n", e);
                continue;
            }
        }

        let r = socket.flush().await;
        if let Err(e) = r {
            println!("web_serve_loop flush error: {:?}\r\n", e);
        }
        Timer::after(Duration::from_millis(500)).await;
        socket.close();
        Timer::after(Duration::from_millis(500)).await;
        socket.abort();
    }
}

#[embassy_executor::task]
async fn connection(mut controller: WifiController<'static>) {
    println!("start connection task\r\n");
    println!("Device capabilities: {:#?}\r\n", controller.capabilities());
    loop {
        if esp_wifi::wifi::wifi_state() == WifiState::StaConnected {
            // wait until we're no longer connected
            controller.wait_for_event(WifiEvent::StaDisconnected).await;
            Timer::after(Duration::from_millis(5000)).await
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = Configuration::Client(ClientConfiguration {
                ssid: SSID.try_into().unwrap(),
                password: PASSWORD.try_into().unwrap(),
                ..Default::default()
            });
            controller.set_configuration(&client_config).unwrap();
            println!("Starting wifi\r\n");
            controller.start_async().await.unwrap();
            println!("Wifi started!\r\n");
        }
        println!("About to connect...\r\n");

        match controller.connect_async().await {
            Ok(_) => println!("Wifi connected!\r\n"),
            Err(e) => {
                println!("Failed to connect to wifi: {e:?}\r\n");
                Timer::after(Duration::from_millis(5000)).await
            }
        }
    }
}

#[embassy_executor::task]
async fn net_task(stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>) {
    stack.run().await
}
