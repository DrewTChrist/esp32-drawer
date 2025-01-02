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
    wifi::{WifiDevice, WifiStaDevice},
    EspWifiController,
};

/// Crate imports
mod tasks;
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

async fn close_socket<'a>(socket: &mut TcpSocket<'a>) {
    let r = socket.flush().await;
    if let Err(e) = r {
        println!("web_serve_loop flush error: {:?}\r\n", e);
    }
    Timer::after(Duration::from_millis(500)).await;
    socket.close();
    Timer::after(Duration::from_millis(500)).await;
    socket.abort();
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

    spawner.spawn(tasks::connection(controller)).ok();
    spawner.spawn(tasks::net_task(stack)).ok();

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

    spawner.spawn(tasks::web::task_loop(stack)).ok();
    spawner.spawn(tasks::backend::task_loop(stack)).ok();

    loop {
        Timer::after(Duration::from_millis(500)).await;
    }
}
