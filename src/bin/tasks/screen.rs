use embassy_sync::{blocking_mutex::raw::NoopRawMutex, signal::Signal};
use embedded_graphics::{
    geometry::Point,
    pixelcolor::{raw::RawU16, Rgb565},
    prelude::*,
    primitives::Rectangle,
};
use embedded_hal_bus::spi::{ExclusiveDevice, NoDelay};
use esp_hal::{gpio::Output, spi::master::Spi, Async};
use esp_println::println;
use st7735_lcd::ST7735;

use esp32_drawer::ScreenSignal;

#[embassy_executor::task]
pub async fn task_loop(
    mut st7735: ST7735<
        ExclusiveDevice<Spi<'static, Async>, Output<'static>, NoDelay>,
        Output<'static>,
        Output<'static>,
    >,
    signal: &'static Signal<NoopRawMutex, ScreenSignal>,
) {
    println!("Starting screen loop\r\n");
    let _color = RawU16::from(Rgb565::RED).into_inner();
    loop {
        let result = signal.wait().await;
        match result {
            ScreenSignal::Coordinate(coordinates) => {
                // draw to screen
                for coordinate in coordinates.into_iter().flatten() {
                    let x = coordinate.1 * 2;
                    let y = coordinate.0 * 2;
                    let rect = Rectangle::new(Point::new(x as i32, y as i32), Size::new(2, 2));
                    if let Err(e) = st7735.fill_solid(&rect, Rgb565::RED) {
                        println!("Error writing pixel to screen: {:?}", e);
                    }
                }
                signal.reset();
            }
            ScreenSignal::Clear => {
                let _ = st7735.clear(Rgb565::BLACK);
                signal.reset();
            }
        }
    }
}
