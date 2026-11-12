//! Single outbound host connection; each peer owns its authenticated Noise state.
use super::{heartbeat::Heartbeat, store::Peer};
use crate::state::AppState;
use anyhow::{Context, Result, ensure};
use futures::{SinkExt, StreamExt};
use neko_remoted_protocol::{self as protocol, messages::RelayFrame};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    sync::{Semaphore, mpsc},
    time::{interval, timeout},
};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
use tokio_util::sync::CancellationToken;

pub struct Link {
    pub peer: String,
    pub outgoing: mpsc::Sender<RelayFrame>,
    pub credits: Arc<Semaphore>,
    pub cancel: CancellationToken,
}
impl Link {
    pub async fn send(&self, bytes: &[u8]) -> Result<()> {
        let permit = tokio::select! {biased;()=self.cancel.cancelled()=>anyhow::bail!("connection closed"),p=self.credits.clone().acquire_owned()=>p?};
        let frame = RelayFrame::Data {
            peer: self.peer.clone(),
            data: protocol::encode(bytes),
            sequence: None,
        };
        tokio::select! {biased;()=self.cancel.cancelled()=>anyhow::bail!("connection closed"),result=self.outgoing.send(frame)=>result?};
        permit.forget();
        Ok(())
    }
}
impl Drop for Link {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
pub enum Input {
    Hello(protocol::crypto::Mode),
    Data(Vec<u8>),
}
struct Inbound {
    tx: mpsc::Sender<Input>,
    credits: Arc<Semaphore>,
    cancel: CancellationToken,
}
pub fn start(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut configuration = state.remoted.configuration.subscribe();
        let mut backoff = 1;
        loop {
            let Ok(saved) = state.remoted.snapshot() else {
                return;
            };
            if !saved.enabled || saved.relay.is_none() {
                status(&state, "disabled", "");
                if configuration.changed().await.is_err() {
                    return;
                }
                continue;
            }
            status(&state, "connecting", "");
            let cancelled = CancellationToken::new();
            let result = tokio::select! {biased;changed=configuration.changed()=>{if changed.is_err(){return;}Ok(())},r=serve(Arc::clone(&state),saved,cancelled.clone())=>r};
            cancelled.cancel();
            if let Ok(mut rt) = state.remoted.runtime.lock() {
                rt.pending.clear();
                rt.invitation = None;
                rt.relay_rtt = None;
                for peer in rt.peers.values() {
                    peer.cancel.cancel();
                }
                rt.peers.clear();
            }
            match result {
                Ok(()) => {
                    backoff = 1;
                }
                Err(e) => {
                    status(&state, "reconnecting", &e.to_string());
                    let jitter = u64::from(protocol::random32().map_or(0, |r| r[0])) * 3;
                    tokio::select! {() = tokio::time::sleep(Duration::from_millis(backoff*1000+jitter))=>{},r=configuration.changed()=>if r.is_err(){return;}}
                    backoff = (backoff * 2).min(30);
                }
            }
        }
    })
}
fn status(state: &AppState, value: &str, error: &str) {
    if let Ok(mut rt) = state.remoted.runtime.lock() {
        rt.status = value.into();
        rt.error = error.into();
        if value != "online" {
            rt.relay_rtt = None;
        }
    }
}
async fn serve(
    state: Arc<AppState>,
    saved: super::store::Saved,
    cancel: CancellationToken,
) -> Result<()> {
    let relay = saved.relay.as_ref().context("relay missing")?;
    let mut request = relay
        .socket("host", &saved.route()?)?
        .into_client_request()?;
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {}", saved.routing_secret).parse()?,
    );
    if let Some(key) = &saved.registration_key {
        request.headers_mut().insert(
            "x-remoted-registration-key",
            key.parse().context("invalid Relay registration key")?,
        );
    }
    request
        .headers_mut()
        .insert("sec-websocket-protocol", "neko.remoted.v1".parse()?);
    let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(protocol::MAX_RELAY_MESSAGE))
        .max_frame_size(Some(protocol::MAX_RELAY_MESSAGE));
    let (mut socket, response) = timeout(
        Duration::from_secs(10),
        tokio_tungstenite::connect_async_with_config(request, Some(config), false),
    )
    .await
    .context("Relay连接超时")??;
    ensure!(
        response
            .headers()
            .get("sec-websocket-protocol")
            .is_some_and(|v| v == "neko.remoted.v1"),
        "Relay协议版本不匹配"
    );
    let (tx, mut rx) = mpsc::channel::<RelayFrame>(128);
    let mut peers: HashMap<String, Inbound> = HashMap::new();
    let mut tasks = tokio::task::JoinSet::new();
    let mut ticks = interval(Duration::from_secs(10));
    let mut heartbeat = Heartbeat::default();
    let mut registered = false;
    loop {
        tokio::select! {
                ()=cancel.cancelled()=>return Ok(()),
                Some(done)=tasks.join_next(),if !tasks.is_empty()=>{if let Ok(peer)=done{if let Some(p)=peers.remove(&peer){p.cancel.cancel();}
        if let Ok(mut rt)=state.remoted.runtime.lock(){rt.peers.remove(&peer);}let _=tx.try_send(RelayFrame::Closed{peer});}},
                Some(frame)=rx.recv()=>{let text=serde_json::to_string(&frame)?;ensure!(text.len()<=protocol::MAX_RELAY_MESSAGE,"relay message too large");timeout(Duration::from_secs(10),socket.send(Message::Text(text.into()))).await.context("Relay发送超时")??;},
                message=socket.next()=>{
                 let message=message.context("Relay连接已经断开")??;
                 let frame=match message{Message::Text(t)=>serde_json::from_str::<RelayFrame>(&t)?,Message::Ping(b)=>{timeout(Duration::from_secs(5),socket.send(Message::Pong(b))).await??;continue;},Message::Pong(_)=>continue,Message::Close(_)=>anyhow::bail!("Relay连接已经断开"),_=>anyhow::bail!("Relay协议消息错误")};
                 match frame{
                  RelayFrame::Connected{peer}=>{ensure!(!registered&&peer=="host","unexpected relay registration");registered=true;status(&state,"online","");},
                  RelayFrame::Opened{peer}=>{
                   ensure!(registered,"relay peer before host registration");
                   if peers.len()>=protocol::MAX_PEERS||peers.contains_key(&peer){let _=tx.try_send(RelayFrame::Closed{peer});continue;}
                   let(input,incoming)=mpsc::channel(8);let token=cancel.child_token();let credits=Arc::new(Semaphore::new(8));
                   let link=Link{peer:peer.clone(),outgoing:tx.clone(),credits:credits.clone(),cancel:token.clone()};
                   peers.insert(peer.clone(),Inbound{tx:input,credits,cancel:token.clone()});
                   if let Ok(mut rt)=state.remoted.runtime.lock(){rt.peers.insert(peer.clone(),Peer{device:None,cancel:token,rtt:None});}
                   let s=Arc::clone(&state);tasks.spawn(async move{let _=super::peer::run(&s,&link,incoming).await;link.cancel.cancel();link.peer.clone()});
                  },
                  RelayFrame::Hello{peer,mode}=>if let Some(p)=peers.get(&peer) && p.tx.try_send(Input::Hello(mode)).is_err(){p.cancel.cancel();},
                  RelayFrame::Data{peer,data,sequence:Some(sequence)}=>{
                   if let Some(p)=peers.get(&peer){match protocol::decode(&data,protocol::MAX_CIPHERTEXT){Ok(bytes)=>if p.tx.try_send(Input::Data(bytes)).is_err(){p.cancel.cancel();},Err(_)=>p.cancel.cancel()}}
                   tx.try_send(RelayFrame::Acknowledge{peer:"host".into(),sequence}).context("relay control queue full")?;
                  },
                  RelayFrame::Delivered{peer,count}=>if let Some(p)=peers.get(&peer){if count>0&&p.credits.available_permits()+usize::from(count)<=8{p.credits.add_permits(usize::from(count));}else{p.cancel.cancel();}},
                  RelayFrame::Closed{peer}=>if let Some(p)=peers.get(&peer){p.cancel.cancel();},
                  RelayFrame::Pong{nonce}=>if let Some(rtt)=heartbeat.pong(&nonce) && let Ok(mut rt)=state.remoted.runtime.lock(){rt.relay_rtt=Some((rtt,Instant::now()));},
                  RelayFrame::Error{code}=>anyhow::bail!("Relay拒绝连接：{code}"),
                  _=>anyhow::bail!("Relay消息顺序错误"),
                 }
                },
                _=ticks.tick()=>{
                 if let Some(nonce)=heartbeat.tick()?{tx.try_send(RelayFrame::Ping{nonce})?;}
                },
               }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Uses a fresh random host route and an in-memory database, never saved user bindings.
    #[tokio::test]
    #[ignore = "requires REMOTED_TEST_RELAY with a reachable deployed Relay"]
    async fn deployed_relay_host_registers_and_measures_rtt() {
        crate::tls::initialize();
        let address = std::env::var("REMOTED_TEST_RELAY").expect("REMOTED_TEST_RELAY required");
        let state = Arc::new(AppState::new_for_test(":memory:".into()));
        let mut saved = state.remoted.snapshot().unwrap();
        saved.enabled = true;
        saved.relay = Some(protocol::RelayAddress::parse(&address).unwrap());
        state.remoted.persist(saved).unwrap();
        let mut worker = start(Arc::clone(&state));
        let connected = timeout(Duration::from_secs(15), async {
            loop {
                tokio::select! {
                    result = &mut worker => panic!("Relay worker exited before registering: {result:?}"),
                    () = tokio::time::sleep(Duration::from_millis(20)) => {
                        let runtime = state.remoted.runtime.lock().unwrap();
                        if runtime.status == "online" && runtime.relay_rtt.is_some() {
                            break;
                        }
                    }
                }
            }
        }).await;
        worker.abort();
        connected.expect("Host must become online and receive a Relay pong");
    }
}
