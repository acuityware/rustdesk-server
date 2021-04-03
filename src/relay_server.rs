use hbb_common::{
    log,
    protobuf::Message as _,
    rendezvous_proto::*,
    sleep,
    tcp::{new_listener, FramedStream},
    tokio::{
        self,
        net::TcpListener,
        time::{interval, Duration},
    },
    ResultType,
};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

lazy_static::lazy_static! {
    static ref PEERS: Arc<Mutex<HashMap<String, FramedStream>>> = Arc::new(Mutex::new(HashMap::new()));
}

pub const DEFAULT_PORT: &'static str = "21117";

#[tokio::main(basic_scheduler)]
pub async fn start(port: &str, key: &str, stop: Arc<Mutex<bool>>) -> ResultType<()> {
    if !key.is_empty() {
        log::info!("Key: {}", key);
    }
    let addr = format!("0.0.0.0:{}", port);
    log::info!("Listening on tcp {}", addr);
    let mut listener = new_listener(addr, false).await?;
    loop {
        if *stop.lock().unwrap() {
            sleep(0.1).await;
            continue;
        }
        log::info!("Start");
        io_loop(&mut listener, key, stop.clone()).await;
    }
}

async fn io_loop(listener: &mut TcpListener, key: &str, stop: Arc<Mutex<bool>>) {
    let mut timer = interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            Ok((stream, addr)) = listener.accept() => {
                let key = key.to_owned();
                tokio::spawn(async move {
                    make_pair(FramedStream::from(stream), addr, &key).await.ok();
                });
            }
            _ = timer.tick() => {
                if *stop.lock().unwrap() {
                    log::info!("Stopped");
                    break;
                }
            }
        }
    }
}

async fn make_pair(stream: FramedStream, addr: SocketAddr, key: &str) -> ResultType<()> {
    let mut stream = stream;
    if let Some(Ok(bytes)) = stream.next_timeout(30_000).await {
        if let Ok(msg_in) = RendezvousMessage::parse_from_bytes(&bytes) {
            if let Some(rendezvous_message::Union::request_relay(rf)) = msg_in.union {
                if !key.is_empty() && rf.licence_key != key {
                    return Ok(());
                }
                if !rf.uuid.is_empty() {
                    let peer = PEERS.lock().unwrap().remove(&rf.uuid);
                    if let Some(peer) = peer {
                        log::info!("Forward request {} from {} got paired", rf.uuid, addr);
                        return relay(stream, peer).await;
                    } else {
                        log::info!("New relay request {} from {}", rf.uuid, addr);
                        PEERS.lock().unwrap().insert(rf.uuid.clone(), stream);
                        sleep(30.).await;
                        PEERS.lock().unwrap().remove(&rf.uuid);
                    }
                }
            }
        }
    }
    Ok(())
}

async fn relay(stream: FramedStream, peer: FramedStream) -> ResultType<()> {
    let mut peer = peer;
    let mut stream = stream;
    peer.set_raw();
    stream.set_raw();
    loop {
        tokio::select! {
            res = peer.next() => {
                if let Some(Ok(bytes)) = res {
                    stream.send_bytes(bytes.into()).await?;
                } else {
                    break;
                }
            },
            res = stream.next() => {
                if let Some(Ok(bytes)) = res {
                    peer.send_bytes(bytes.into()).await?;
                } else {
                    break;
                }
            },
        }
    }
    Ok(())
}
