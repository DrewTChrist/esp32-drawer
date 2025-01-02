#![no_std]

pub mod buffer;

use core::fmt::Write as CoreWrite;
use embassy_net::tcp::TcpSocket;
use embassy_time::{Duration, Timer};
use embedded_io_async::Write;
use esp_println::println;

pub async fn get_request<'a, 'b>(
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

pub fn write_response_status<const S: usize>(
    response_buffer: &mut buffer::ResponseBuffer<S>,
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

pub fn write_response_headers<const S: usize>(response_buffer: &mut buffer::ResponseBuffer<S>) {
    if let Err(e) = write!(response_buffer, "Access-Control-Allow-Origin: *\r\n") {
        println!("Error writing response headers: {:?}", e);
    }
    if let Err(e) = write!(response_buffer, "\r\n") {
        println!("Error writing response headers: {:?}", e);
    }
}

pub async fn send_response_buffer<'a, const S: usize>(
    socket: &mut TcpSocket<'a>,
    response_buffer: buffer::ResponseBuffer<S>,
) {
    if let Err(e) = socket.write_all(response_buffer.buffer()).await {
        println!("AP write error: {:?}\r\n", e);
    }
}

pub async fn close_socket<'a>(socket: &mut TcpSocket<'a>) {
    let r = socket.flush().await;
    if let Err(e) = r {
        println!("web_serve_loop flush error: {:?}\r\n", e);
    }
    Timer::after(Duration::from_millis(500)).await;
    socket.close();
    Timer::after(Duration::from_millis(500)).await;
    socket.abort();
}
