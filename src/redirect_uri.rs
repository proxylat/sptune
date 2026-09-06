use std::{
  io::{BufRead, BufReader, Write},
  net::{TcpListener, TcpStream},
};

pub fn redirect_uri_web_server(port: u16) -> Result<String, ()> {
  let listener = match TcpListener::bind(format!("127.0.0.1:{}", port)) {
    Ok(l) => l,
    Err(e) => {
      println!("Error: failed to bind 127.0.0.1:{} — {}", port, e);
      return Err(());
    }
  };

  for stream in listener.incoming() {
    match stream {
      Ok(stream) => {
        if let Some(url) = handle_connection(stream) {
          return Ok(url);
        }
      }
      Err(e) => {
        println!("Error: {}", e);
      }
    }
  }
  Err(())
}

fn handle_connection(stream: TcpStream) -> Option<String> {
  let mut reader = BufReader::new(&stream);
  let mut request_line = String::new();
  if reader.read_line(&mut request_line).is_err() {
    respond_with_error("Failed to read request".to_string(), stream);
    return None;
  }
  // Drain remaining headers so browser doesn't hang
  let mut line = String::new();
  loop {
    line.clear();
    match reader.read_line(&mut line) {
      Ok(0) | Err(_) => break,
      Ok(_) if line == "\r\n" || line == "\n" => break,
      _ => {}
    }
  }
  drop(reader);

  let split: Vec<&str> = request_line.split_whitespace().collect();
  if split.len() < 2 {
    respond_with_error("Malformed request".to_string(), stream);
    return None;
  }
  let path = split[1].to_string();
  // Ignore browser noise like /favicon.ico — keep listening for /callback
  if !path.starts_with("/callback") {
    respond_with_success(stream);
    return None;
  }
  // Spotify can redirect with ?error=access_denied if user cancels
  if path.contains("error=") {
    respond_with_error(format!("Spotify returned error: {}", path), stream);
    return Some(path);
  }
  respond_with_success(stream);
  Some(path)
}

fn respond_with_success(mut stream: TcpStream) {
  let contents = include_str!("redirect_uri.html");
  let response = format!(
    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
    contents.len(),
    contents
  );
  let _ = stream.write_all(response.as_bytes());
  let _ = stream.flush();
}

fn respond_with_error(error_message: String, mut stream: TcpStream) {
  println!("Error: {}", error_message);
  let body = format!("400 - Bad Request - {}", error_message);
  let response = format!(
    "HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
    body.len(),
    body
  );
  let _ = stream.write_all(response.as_bytes());
  let _ = stream.flush();
}
