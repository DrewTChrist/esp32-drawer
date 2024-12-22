//!

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

use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;

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

#[derive(Clone, Copy, Serialize, Deserialize)]
struct Row {
    #[serde(with = "BigArray")]
    data: [u8; 10],
}

#[derive(Serialize, Deserialize)]
struct GridData {
    #[serde(with = "BigArray")]
    data: [Row; 10],
}

#[derive(Debug)]
enum WebServeFile<'a> {
    File(&'a [u8]),
    NotFound,
}

const INDEX: WebServeFile<'static> = WebServeFile::File(include_bytes!("../index.html"));
const CSS: WebServeFile<'static> = WebServeFile::File(include_bytes!("../css/style.css"));

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

async fn get_request<'a, 'b>(socket: &mut TcpSocket<'a>, buffer: &'b mut [u8]) -> &'b str {
    let mut pos = 0;
    loop {
        match socket.read(buffer).await {
            Ok(0) => {
                println!("AP read EOF\r\n");
                break;
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
            Err(e) => {
                println!("AP read error: {:?}\r\n", e);
                break;
            }
        };
    }
    core::str::from_utf8(buffer).unwrap_or(&"")
}

async fn send_response_status<'a>(socket: &mut TcpSocket<'a>, status_code: usize) {
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
    buffer: ResponseBuffer<S>,
) {
    if let Err(e) = socket.write_all(&buffer.headers).await {
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
        init(timg0.timer0, rng.clone(), peripherals.RADIO_CLK).unwrap()
    );

    let wifi = peripherals.WIFI;
    let (wifi_interface, controller) =
        esp_wifi::wifi::new_with_mode(&init, wifi, WifiStaDevice).unwrap();

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
    spawner.spawn(net_task(&stack)).ok();

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

    spawner.spawn(web_serve_loop(&stack)).ok();
    spawner.spawn(backend_loop(&stack)).ok();

    loop {
        Timer::after(Duration::from_millis(500)).await;
    }
}

#[embassy_executor::task]
async fn backend_loop(stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>) {
    println!("Starting backend_loop\r\n");
    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];
    let mut socket = TcpSocket::new(&stack, &mut rx_buffer, &mut tx_buffer);
    socket.set_timeout(Some(embassy_time::Duration::from_secs(10)));

    let grid_data = GridData {
        data: [Row { data: [0; 10] }; 10],
    };

    loop {
        let r = socket.accept(BACKEND_ENDPOINT).await;

        if let Err(e) = r {
            println!("connect error: {:?}\r\n", e);
            continue;
        }

        let mut buffer = [0u8; 512];
        let mut response_buffer = ResponseBuffer::<256>::new();
        let request_content = get_request(&mut socket, &mut buffer).await;

        // println!("request_content: {:?}", request_content);
        let mut lines = request_content.split("\r\n");

        let first_line = lines.nth(0).unwrap_or(&"");
        let mut parts = first_line.split(' ');
        let method = parts.nth(0).unwrap_or(&"");
        let path = parts.nth(0).unwrap_or(&"");

        println!("backend_loop: {:?} {:?}\r\n", method, path);

        match method {
            "GET" => match path {
                "/data" => {
                    let mut buffer = [0; (50 * 50) + 1024];
                    match serde_json_core::to_slice(&grid_data, &mut buffer[..]) {
                        Ok(len) => {
                            write_response_status(&mut response_buffer, 200);
                            write_response_headers(&mut response_buffer);
                            if let Err(e) = socket.write_all(&buffer[..len]).await {
                                println!("AP write error: {:?}\r\n", e);
                            }
                            // println!("Bytes converted: {:?}\r\n", len);
                        }
                        Err(_) => {
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
                    println!("There is data to receive\r\n");
                    write_response_status(&mut response_buffer, 200);
                    write_response_headers(&mut response_buffer);
                }
                "/clear" => {
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
    let mut socket = TcpSocket::new(&stack, &mut rx_buffer, &mut tx_buffer);
    // socket.set_timeout(Some(embassy_time::Duration::from_secs(10)));

    loop {
        println!("Wait for connection...\r\n");

        let r = socket.accept(WEB_ENDPOINT).await;

        println!("Connected...\r\n");

        if let Err(e) = r {
            println!("connect error: {:?}\r\n", e);
            continue;
        }

        let mut buffer = [0u8; 512];
        let request_content = get_request(&mut socket, &mut buffer).await;

        let mut lines = request_content.split("\r\n");

        let first_line = lines.nth(0).unwrap_or(&"");
        let mut parts = first_line.split(' ');
        let method = parts.nth(0).unwrap_or(&"");
        let path = parts.nth(0).unwrap_or(&"");

        match method {
            "GET" => match path_to_file(path) {
                WebServeFile::File(contents) => {
                    send_response_status(&mut socket, 200).await;
                    if let Err(e) = socket.write_all(contents).await {
                        println!("AP write error: {:?}\r\n", e);
                    }
                }
                WebServeFile::NotFound => send_response_status(&mut socket, 404).await,
            },
            _ => send_response_status(&mut socket, 404).await,
        }

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
async fn connection(mut controller: WifiController<'static>) {
    println!("start connection task\r\n");
    println!("Device capabilities: {:#?}\r\n", controller.capabilities());
    loop {
        match esp_wifi::wifi::wifi_state() {
            WifiState::StaConnected => {
                // wait until we're no longer connected
                controller.wait_for_event(WifiEvent::StaDisconnected).await;
                Timer::after(Duration::from_millis(5000)).await
            }
            _ => {}
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
