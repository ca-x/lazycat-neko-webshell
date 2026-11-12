//! Two isolated private LANs behind RFC 4787 NATs, using upstream's VNet API.
//! This is deterministic network emulation, not an Internet/NAT field test.
use anyhow::Result;
use std::{net::SocketAddr, sync::Arc};
use tokio::{sync::Mutex, task::JoinHandle};
use webrtc::{
    stun::{
        message::{BINDING_REQUEST, BINDING_SUCCESS, Message},
        xoraddr::XorMappedAddress,
    },
    util::vnet::{
        nat::{EndpointDependencyType, NatType},
        net::{Net, NetConfig},
        router::{Nic, Router, RouterConfig},
    },
};

pub struct Network {
    pub peers: [Arc<Net>; 2],
    wan: Arc<Mutex<Router>>,
    stun: JoinHandle<Result<()>>,
    _stun_net: Arc<Net>,
}

async fn connect(net: &Arc<Net>, router: &Arc<Mutex<Router>>) -> Result<()> {
    let nic = net.get_nic()?;
    router.lock().await.add_net(nic.clone()).await?;
    nic.lock().await.set_router(router.clone()).await?;
    Ok(())
}

impl Network {
    pub async fn new(symmetric: bool, full_cone: bool) -> Result<Self> {
        let wan = Arc::new(Mutex::new(Router::new(RouterConfig {
            cidr: "0.0.0.0/0".to_owned(),
            ..Default::default()
        })?));
        let stun_net = Arc::new(Net::new(Some(NetConfig {
            static_ip: "1.2.3.4".to_owned(),
            ..Default::default()
        })));
        connect(&stun_net, &wan).await?;
        let mut peers = Vec::new();
        for (private, public) in [("192.168.0.1", "27.1.1.1"), ("10.2.0.1", "28.1.1.1")] {
            let router = Arc::new(Mutex::new(Router::new(RouterConfig {
                cidr: format!("{private}/24"),
                static_ips: vec![public.to_owned()],
                nat_type: Some(NatType {
                    mapping_behavior: if symmetric {
                        EndpointDependencyType::EndpointAddrPortDependent
                    } else {
                        EndpointDependencyType::EndpointIndependent
                    },
                    filtering_behavior: if full_cone {
                        EndpointDependencyType::EndpointIndependent
                    } else {
                        EndpointDependencyType::EndpointAddrPortDependent
                    },
                    ..Default::default()
                }),
                ..Default::default()
            })?));
            let peer = Arc::new(Net::new(Some(NetConfig {
                static_ips: vec![private.to_owned()],
                ..Default::default()
            })));
            connect(&peer, &router).await?;
            wan.lock().await.add_router(router.clone()).await?;
            router.lock().await.set_router(wan.clone()).await?;
            peers.push(peer);
        }
        wan.lock().await.start().await?;
        // This server supports STUN Binding only. There is no TURN allocation,
        // port forwarding, 1:1 NAT mapping, or public interface on either peer.
        let socket = stun_net.bind("1.2.3.4:3478".parse::<SocketAddr>()?).await?;
        let stun = tokio::spawn(async move {
            let mut buffer = vec![0u8; 2048];
            loop {
                let (length, address) = socket.recv_from(&mut buffer).await?;
                let mut request = Message {
                    raw: buffer[..length].to_vec(),
                    ..Default::default()
                };
                if request.decode().is_err() || request.typ != BINDING_REQUEST {
                    continue;
                }
                let mut response = Message::default();
                response.build(&[
                    Box::new(request.transaction_id),
                    Box::new(BINDING_SUCCESS),
                    Box::new(XorMappedAddress {
                        ip: address.ip(),
                        port: address.port(),
                    }),
                ])?;
                socket.send_to(&response.raw, address).await?;
            }
        });
        let peers: [Arc<Net>; 2] = peers
            .try_into()
            .map_err(|_| anyhow::anyhow!("two peers required"))?;
        Ok(Self {
            peers,
            wan,
            stun,
            _stun_net: stun_net,
        })
    }

    pub async fn close(self) -> Result<()> {
        self.stun.abort();
        self.wan.lock().await.stop().await?;
        Ok(())
    }
}
