//! Narrow remote attachment. Never creates an hc terminal and never elects a controller.
use super::{
    protocol::{CLOSE, CREDIT, DATA, RESIZE},
    session::Session,
};
use crate::state::AppState;
use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

pub(crate) enum Command {
    Input(Vec<u8>, oneshot::Sender<Result<()>>),
    Resize(u16, u16, oneshot::Sender<Result<()>>),
}
pub(crate) struct Remote {
    pub commands: mpsc::Sender<Command>,
    pub output: mpsc::Receiver<Result<Vec<u8>>>,
}
pub(crate) async fn attach(
    state: Arc<AppState>,
    id: &str,
    read_only: bool,
    cancel: CancellationToken,
) -> Result<Remote> {
    let session = Session::load(state, id).await?;
    let mut client = session.connect().await?;
    let terminal = session.resolve(&mut client, false).await?;
    ensure!(terminal["state"] != "finished", "hc终端已经退出");
    let reply=client.call("terminal.attach",json!({"terminal":terminal["address"],"cols":session.record.cols,"rows":session.record.rows,"history":1000,"resumed":true,"observe":read_only,"display":{}})).await?;
    let channel = u32::try_from(reply["channel"].as_u64().context("hc omitted channel")?)?;
    let (tx, mut rx) = mpsc::channel(16);
    let (output, events) = mpsc::channel(8);
    tokio::spawn(async move {
        let result:Result<()>=async{
   loop{tokio::select!{biased;
    ()=cancel.cancelled()=>return Ok(()),
    Some(command)=rx.recv()=>match command{
     Command::Input(data,done)=>{let result=if read_only{Err(anyhow::anyhow!("只读设备不能输入"))}else{client.send(channel,DATA,&data).await};let failed=result.is_err();let _=done.send(result);if failed{return Ok(());}},
     Command::Resize(cols,rows,done)=>{let mut payload=cols.to_le_bytes().to_vec();payload.extend_from_slice(&rows.to_le_bytes());let result=if read_only{Err(anyhow::anyhow!("只读设备不能调整尺寸"))}else{client.send(channel,RESIZE,&payload).await};let failed=result.is_err();let _=done.send(result);if failed{return Ok(());}},
    },
    frame=client.next_frame()=>{
     let frame=frame?.context("hc连接已经断开")?;
     if frame.channel!=channel{continue;}
     if frame.kind==CLOSE{anyhow::bail!("hc终端流已结束：{}",String::from_utf8_lossy(&frame.payload));}
     if frame.kind==DATA{
      for chunk in frame.payload.chunks(neko_remoted_protocol::FILE_CHUNK){tokio::select!{()=cancel.cancelled()=>return Ok(()),r=output.send(Ok(chunk.to_vec()))=>r?};}
      client.send(channel,CREDIT,&u32::try_from(frame.payload.len())?.to_le_bytes()).await?;
     }
    }
   }}
  }.await;
        if let Err(error) = result {
            tokio::select! {
                () = cancel.cancelled() => {},
                _ = output.send(Err(error)) => {},
            }
        }
        let _ = client.close().await;
    });
    Ok(Remote {
        commands: tx,
        output: events,
    })
}
