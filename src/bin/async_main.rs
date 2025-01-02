#![no_std]
#![no_main]

/// External imports
use embassy_executor::Spawner;
use embassy_net::{IpListenEndpoint, Stack, StackResources};
use embassy_time::{Duration, Timer};
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
