use crate::highlight;
use crate::vim::buffer::Buffer;
use anyhow::{anyhow, Result};
use log::*;
use neovim_lib::Value;
use std::io::Write;
use std::sync::Arc;
use tempfile::NamedTempFile;
use tokio::sync::mpsc;
use tokio::sync::Mutex;

pub fn parse_string(value: &neovim_lib::Value) -> Result<String> {
  value
    .as_str()
    .ok_or(anyhow!("cannot parse error"))
    .map(|s| String::from(s))
}

pub fn parse_usize(value: &neovim_lib::Value) -> Result<usize> {
  value
    .as_u64()
    .ok_or(anyhow!("cannot parse usize"))
    .map(|n| n as usize)
}

#[derive(Debug)]
pub enum Event {
  Apply { buffer: Buffer },
  OpenLog,
}

pub struct Handler {
  runtime_handle: tokio::runtime::Handle,
  event_sender: Arc<Mutex<mpsc::UnboundedSender<Event>>>,
}

impl Handler {
  pub fn new(
    event_sender: mpsc::UnboundedSender<Event>,
    runtime_handle: tokio::runtime::Handle,
  ) -> Self {
    Self {
      runtime_handle,
      event_sender: Arc::new(Mutex::new(event_sender)),
    }
  }

  async fn push(runtime_handle: tokio::runtime::Handle, args: &Vec<Value>) -> Result<Event> {
    if args.len() != 3 {
      return Err(anyhow!("invalid args to push: {:?}", args));
    }

    let filename = parse_string(&args[0])?;
    let filetype = parse_string(&args[1])?;
    let data = parse_string(&args[2])?;

    let mut temp_file = NamedTempFile::new()?;
    temp_file.write_all(data.as_bytes())?;
    let tokens;
    {
      tokens = runtime_handle
        .spawn_blocking(move || crate::clang::tokenize(temp_file))
        .await
        .unwrap()?;
    }

    Ok(Event::Apply {
      buffer: Buffer::new(&filename, highlight::Group::new(tokens)),
    })
  }
}

impl neovim_lib::Handler for Handler {
  fn handle_notify(&mut self, name: &str, args: Vec<Value>) {
    debug!("handler event: {}", name);
    match name {
      /* TODO: Queue. */
      "enter_buffer" | "push" => {
        let event_sender = self.event_sender.clone();
        let runtime_handle = self.runtime_handle.clone();
        self.runtime_handle.spawn(async move {
          match Self::push(runtime_handle, &args).await {
            Ok(event) => {
              let event_sender = event_sender.lock().await;
              if let Err(reason) = event_sender.send(event) {
                // TODO: Improve
                error!("failed to send {}", reason);
              }
            }
            Err(e) => error!("failed to push: {}", e),
          }
        });
      }
      "open_log" => {
        let event_sender = self.event_sender.clone();
        self.runtime_handle.spawn(async move {
          let event_sender = event_sender.lock().await;
          if let Err(reason) = event_sender.send(Event::OpenLog) {
            // TODO: Improve
            error!("failed to send {}", reason);
          }
        });
      }
      _ => {
        debug!("unmatched event: {}", name);
      }
    }
  }
}

impl neovim_lib::RequestHandler for Handler {
  fn handle_request(&mut self, _name: &str, _args: Vec<Value>) -> Result<Value, Value> {
    Err(Value::from("not implemented"))
  }
}
