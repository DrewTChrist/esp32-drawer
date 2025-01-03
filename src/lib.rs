#![no_std]

pub mod buffer;

use core::fmt::Write as CoreWrite;
use embassy_net::tcp::TcpSocket;
use embassy_time::{Duration, Timer};
use embedded_io_async::Write;
use esp_println::println;

use crate::buffer::RequestBuffer;

#[derive(Clone, Copy)]
pub struct Request<'a, const S: usize> {
    buffer: Option<&'a RequestBuffer<S>>,
    rb: Option<&'a [u8]>,
    pub method: Option<&'a str>,
    pub path: Option<&'a str>,
    pub headers: [Option<&'a str>; 32],
    pub data: Option<&'a str>,
}

impl<'a, const S: usize> Default for Request<'a, S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, const S: usize> Request<'a, S> {
    pub fn new() -> Self {
        Self {
            buffer: None,
            rb: None,
            method: None,
            path: None,
            headers: [None; 32],
            data: None,
        }
    }

    pub fn set_request_buffer(&mut self, buffer: &'a RequestBuffer<S>) {
        self.buffer = Some(buffer);
        self.set_buffer();
    }

    fn set_buffer(&mut self) {
        self.rb = Some(&self.buffer.unwrap().buf);
    }

    pub fn parse_request(&mut self) {
        if let Some(buffer) = self.rb {
            if let Ok(result) = core::str::from_utf8(buffer) {
                let mut lines = result.split("\r\n");
                let first_line = lines.next().unwrap_or("");
                let mut parts = first_line.split(' ');
                let method = parts.next().unwrap_or("");
                let path = parts.next().unwrap_or("");
                for (pos, line) in lines.by_ref().enumerate() {
                    if line.is_empty() {
                        break;
                    }
                    self.headers[pos] = Some(line);
                }
                let data = lines.next().unwrap_or("").trim_matches(char::from(0));
                self.method = Some(method);
                self.path = Some(path);
                self.data = Some(data);
            }
        }
    }
}

pub async fn get_request<'a, const S: usize>(
    socket: &mut TcpSocket<'a>,
    request_buffer: &mut RequestBuffer<S>,
) -> Result<(), embassy_net::tcp::Error> {
    let mut pos = 0;
    loop {
        match socket.read(request_buffer.buffer_mut()).await {
            Ok(0) => {
                println!("AP read EOF\r\n");
                return Err(embassy_net::tcp::Error::ConnectionReset);
            }
            Ok(len) => match core::str::from_utf8(&request_buffer.buffer()[..(pos + len)]) {
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
