use iced::futures::channel::mpsc;
use iced::futures::sink::SinkExt;
use iced::futures::Stream;
use iced::stream;
use iced_futures::futures::StreamExt;
use socket2::{SockRef, TcpKeepalive};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;

pub mod proto;
use proto::*;

mod packing;
use packing::*;

const SLSK_SERVER: &str = "server.slsknet.org:2242";

#[derive(Debug, Clone)]
pub enum ServerCommand {
    ResetConnection,
    Login {
        username: String,
        password: String,
    },
    SetWaitPort {
        port: u16,
        obfuscated_port: u16,
    },
    ConnectToPeer {
        token: u32,
        username: String,
        conn_type: ConnectionType,
    },
    FileSearch {
        token: u32,
        search_query: String,
    },
}

#[derive(Debug, Clone)]
pub enum ServerEvent {
    Connected(mpsc::Sender<ServerCommand>),
    ConnectionFailure,

    LoginSuccess(LoginResponseSuccess),
    LoginFailure(LoginResponseFailure),
    ParentMinSpeed(u32),
    ParentSpeedRatio(u32),
    WishlistInterval(u32),

    ConnectToPeer(ConnectToPeerResponse),
    PrivilegedUsers(PrivilegedUsersResponse),
    ExcludedSearchPhrases(ExcludedSearchPhrasesResponse),

    FileSearch(FileSearchResponse),
}

pub fn server_worker() -> impl Stream<Item = ServerEvent> {
    stream::channel(100, |o| async move {
        loop {
            let mut application_output = o.clone();

            let Ok(slsk_stream) = TcpStream::connect(SLSK_SERVER).await else {
                let _ = application_output
                    .send(ServerEvent::ConnectionFailure)
                    .await;
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                continue;
            };
            let ka = TcpKeepalive::new().with_time(std::time::Duration::from_secs(30));
            let sf = SockRef::from(&slsk_stream);
            if sf.set_tcp_keepalive(&ka).is_err() {
                continue;
            }

            let (mut read_stream, mut write_stream) = slsk_stream.into_split();

            let (sender, mut application_input) = mpsc::channel(100);

            if application_output
                .send(ServerEvent::Connected(sender))
                .await
                .is_err()
            {
                continue;
            };

            let reset_token = CancellationToken::new();

            let reader_token = reset_token.child_token();
            let writer_token = reset_token.child_token();

            let reader_handle = tokio::spawn(async move {
                tokio::select! {
                    _ = reader_token.cancelled() => (),
                    _ = server_reader(&mut read_stream, &mut application_output) => ()
                }
            });

            let writer_handle = tokio::spawn(async move {
                tokio::select! {
                    _ = writer_token.cancelled() => (),
                    _ = server_writer(&mut write_stream, &mut application_input, &reset_token) => ()
                }
            });

            let _ = reader_handle.await;
            let _ = writer_handle.await;
        }
    })
}

pub async fn server_reader(
    read_stream: &mut OwnedReadHalf,
    application_output: &mut mpsc::Sender<ServerEvent>,
) {
    loop {
        let Ok(length) = read_stream.read_u32_le().await else {
            continue;
        };
        let code = read_stream.read_u32_le().await.unwrap();
        let mut resp = vec![0; length as usize - 4];
        let _ = read_stream.read_exact(&mut resp).await;

        match code {
            1 => {
                let response = LoginResponse::from_bytes(&resp).unwrap();
                println!("KNOWN_MESSAGE: {:?}", response);
                let response = match response {
                    LoginResponse::Success(resp) => ServerEvent::LoginSuccess(resp),
                    LoginResponse::Failure(resp) => ServerEvent::LoginFailure(resp),
                };
                let _ = application_output.send(response).await;
            }
            18 => {
                let response = ConnectToPeerResponse::from_bytes(&resp).unwrap();
                //println!("KNOWN_MESSAGE: {:?}", response);

                let _ = application_output
                    .send(ServerEvent::ConnectToPeer(response))
                    .await;
            }
            83 => {
                let Ok(speed) = resp.as_slice().unpack_u32() else {
                    continue;
                };
                println!("KNOWN_MESSAGE: ParentMinSpeed({})", speed);
                let _ = application_output
                    .send(ServerEvent::ParentMinSpeed(speed))
                    .await;
            }
            84 => {
                let Ok(speed) = resp.as_slice().unpack_u32() else {
                    continue;
                };
                println!("KNOWN_MESSAGE: ParentSpeedRatio({})", speed);
                let _ = application_output
                    .send(ServerEvent::ParentSpeedRatio(speed))
                    .await;
            }
            104 => {
                let Ok(interval) = resp.as_slice().unpack_u32() else {
                    continue;
                };
                println!("KNOWN_MESSAGE: WishlistInterval({})", interval);
                let _ = application_output
                    .send(ServerEvent::WishlistInterval(interval))
                    .await;
            }
            69 => {
                let response = PrivilegedUsersResponse::from_bytes(&resp).unwrap();
                println!("KNOWN_MESSAGE: {:?}", response);
                let _ = application_output
                    .send(ServerEvent::PrivilegedUsers(response))
                    .await;
            }
            160 => {
                let response = ExcludedSearchPhrasesResponse::from_bytes(&resp).unwrap();
                println!("KNOWN_MESSAGE: {:?}", response);
                let _ = application_output
                    .send(ServerEvent::ExcludedSearchPhrases(response))
                    .await;
            }
            26 => {
                let response = FileSearchResponse::from_bytes(&resp).unwrap();
                println!("KNOWN_MESSAGE: {:?}", response);
                let _ = application_output
                    .send(ServerEvent::FileSearch(response))
                    .await;
            }
            _ => println!("UNKNOWN_MESSAGE ({}): {:02x?}", code, resp),
        }
    }
}

pub async fn server_writer(
    write_stream: &mut OwnedWriteHalf,
    application_input: &mut mpsc::Receiver<ServerCommand>,
    reset_token: &CancellationToken,
) {
    loop {
        match application_input.select_next_some().await {
            ServerCommand::Login { username, password } => {
                println!("SENDING_COMMAND: ServerCommand::Login");
                let request = login_request(username, password).unwrap();
                let _ = write_stream.write_all(request.as_slice()).await;
            }
            ServerCommand::ConnectToPeer {
                token,
                username,
                conn_type,
            } => {
                println!("SENDING_COMMAND: ServerCommand::ConnectToPeer");
                let request = connect_to_peer_request(token, username, conn_type).unwrap();
                let _ = write_stream.write_all(request.as_slice()).await;
            }
            ServerCommand::ResetConnection => {
                reset_token.cancel();
            }
            ServerCommand::SetWaitPort {
                port,
                obfuscated_port,
            } => {
                println!("SENDING_COMMAND: ServerCommand::SetWaitPort");
                let request = set_wait_port_request(port, obfuscated_port).unwrap();
                let _ = write_stream.write_all(request.as_slice()).await;
            }
            ServerCommand::FileSearch {
                token,
                search_query,
            } => {
                println!("SENDING_COMMAND: ServerCommand::SetWaitPort");
                let request = file_search_request(token, search_query).unwrap();
                let _ = write_stream.write_all(request.as_slice()).await;
            }
        }

        let _ = write_stream.flush().await;
        println!("SENDING_COMMAND: Command sent");
    }
}

#[derive(Debug, Clone)]
pub enum PeerCommand {
    Init(std::net::SocketAddrV4),
    Reset,
    PierceFirewall(u32),
}

#[derive(Debug, Clone)]
pub enum PeerEvent {
    Ready(mpsc::Sender<PeerCommand>),
    InitSuccess,
    InitFailed,
    PierceFirewall(PierceFirewallResponse),
    FileSearch(FileSearchResponsePeer),
}

pub fn peer_worker() -> impl Stream<Item = PeerEvent> {
    stream::channel(100, |o| async move {
        loop {
            let mut application_output = o.clone();

            let (sender, mut application_input) = mpsc::channel(100);

            // FIXME: Add error logging here
            let _ = application_output.send(PeerEvent::Ready(sender)).await;

            let peer_stream = loop {
                match application_input.select_next_some().await {
                    PeerCommand::Init(addr) => {
                        let Ok(Ok(peer)) =
                            timeout(Duration::from_millis(150), TcpStream::connect(addr)).await
                        else {
                            let _ = application_output.send(PeerEvent::InitFailed).await;
                            continue;
                        };
                        let _ = application_output.send(PeerEvent::InitSuccess).await;
                        break peer;
                    }
                    _ => (),
                }
            };
            println!("HERE");

            let ka = TcpKeepalive::new().with_time(std::time::Duration::from_secs(30));
            let sf = SockRef::from(&peer_stream);
            if sf.set_tcp_keepalive(&ka).is_err() {
                continue;
            }

            let (mut read_stream, mut write_stream) = peer_stream.into_split();

            let reset_token = CancellationToken::new();

            let reader_token = reset_token.child_token();
            let writer_token = reset_token.child_token();

            let reader_handle = tokio::spawn(async move {
                tokio::select! {
                    _ = reader_token.cancelled() => (),
                    _ = peer_reader(&mut read_stream, &mut application_output) => ()
                }
            });

            let writer_handle = tokio::spawn(async move {
                tokio::select! {
                    _ = writer_token.cancelled() => (),
                    _ = peer_writer(&mut write_stream, &mut application_input, &reset_token) => ()
                }
            });

            let _ = reader_handle.await;
            let _ = writer_handle.await;
        }
    })
}

pub async fn peer_reader(
    read_stream: &mut OwnedReadHalf,
    application_output: &mut mpsc::Sender<PeerEvent>,
) {
    loop {
        let Ok(length) = read_stream.read_u32_le().await else {
            continue;
        };
        let (mut resp, code) = match read_stream.read_u32_le().await {
            Ok(code) => (vec![0; length as usize - 4], Ok(code)),
            Err(_) => (
                vec![0; length as usize - 1],
                read_stream.read_u8().await.map(|n| n as u32),
            ),
        };
        let Ok(code) = code else {
            continue;
        };
        let _ = read_stream.read_exact(&mut resp).await;

        match code {
            0 => {
                let response = PierceFirewallResponse::from_bytes(&resp).unwrap();
                println!("KNOWN_MESSAGE: {:?}", response);
                let _ = application_output
                    .send(PeerEvent::PierceFirewall(response))
                    .await;
            }
            9 => {
                let response = FileSearchResponsePeer::from_bytes(&resp).unwrap();
                println!("KNOWN_MESSAGE: {:?}", response);
                let _ = application_output
                    .send(PeerEvent::FileSearch(response))
                    .await;
            }
            _ => println!("UNKNOWN_PEER__MESSAGE ({}): {:02x?}", code, resp),
        }
    }
}

pub async fn peer_writer(
    write_stream: &mut OwnedWriteHalf,
    application_input: &mut mpsc::Receiver<PeerCommand>,
    reset_token: &CancellationToken,
) {
    loop {
        match application_input.select_next_some().await {
            PeerCommand::PierceFirewall(token) => {
                println!("SENDING_COMMAND: PeerCommand::PierceFirewall");
                let request = pierce_firewall_request(token).unwrap();
                let _ = write_stream.write_all(request.as_slice()).await;
            }
            PeerCommand::Reset => reset_token.cancel(),
            _ => (),
        }
    }
}
