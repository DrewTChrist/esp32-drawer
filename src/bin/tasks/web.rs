/// Core imports
use core::fmt::Write;
use embassy_net::{tcp::TcpSocket, Stack};
use embassy_time::{Duration, Timer};
use embedded_io_async::Write as EmbeddedIoWrite;
use esp_println::println;
use esp_wifi::wifi::{WifiDevice, WifiStaDevice};

/// Crate imports
use crate::close_socket;
use crate::get_request;
use crate::path_to_file;
use crate::send_response_buffer;
use crate::write_response_headers;
use crate::write_response_status;
use crate::WebServeFile;
use crate::WEB_ENDPOINT;
use esp32_drawer::buffer::ResponseBuffer;

#[embassy_executor::task]
// async fn web_serve_loop(stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>) {
pub async fn task_loop(stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>) {
    println!("Starting web_serve_loop\r\n");
    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];
    let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
    socket.set_timeout(Some(embassy_time::Duration::from_secs(10)));

    loop {
        let r = socket.accept(WEB_ENDPOINT).await;

        if let Err(e) = r {
            // close the socket if it is in at invalid state
            println!("connect error: {:?}\r\n", e);
            close_socket(&mut socket).await;
            continue;
        }

        let mut buffer = [0u8; 512];
        let mut response_buffer = ResponseBuffer::<512>::new();
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
