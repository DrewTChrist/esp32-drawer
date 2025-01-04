pub mod backend;
pub mod screen;
pub mod web;

use embassy_net::Stack;
use embassy_time::{Duration, Timer};
// use esp_hal::cpu_control::Stack;
use esp_println::println;
use esp_wifi::wifi::{
    ClientConfiguration, Configuration, WifiController, WifiDevice, WifiEvent, WifiStaDevice,
    WifiState,
};

use crate::PASSWORD;
use crate::SSID;

#[embassy_executor::task]
pub async fn connection(mut controller: WifiController<'static>) {
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
pub async fn net_task(stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>) {
    stack.run().await
}
