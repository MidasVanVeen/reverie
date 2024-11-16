use std::collections::{HashMap, VecDeque};
use std::net::Ipv4Addr;
use std::time::Duration;

use iced::futures::channel::mpsc;
use iced::futures::SinkExt;
use iced::widget::container;
use iced::Color;
use iced::Length::Fill;
use iced::{
    widget::{button, column, row, text, text_input, scrollable},
    Element, Subscription, Task,
};

mod error;
use error::Result;

mod soulseek;
use soulseek::proto::*;
use soulseek::*;
use tokio::time::Instant;

const LISTENER_PORT: u16 = 2234;
const OBFUSCATED_PORT: u16 = 2245;
const MAX_PEER_TTL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone)]
enum ReverieMessage {
    Server(ServerEvent),
    Peer(PeerEvent),
    NextPeer,
    Periodic,
    SentCommand,

    // Inputs
    LoginPressed,
    UsernameInputChange(String),
    PasswordInputChange(String),
    SearchQueryChange(String),
    SearchPressed,
}

#[derive(Default, Debug)]
struct ServerState {
    connected: bool,
    logged_in: bool,
    latest_login_failure: Option<LoginFailureReason>,
}

#[derive(Default, Debug)]
struct PeerState {
    expected_token: u32,

    active_peer: Option<ConnectToPeerResponse>,
    active_since: Option<Instant>,

    privileged_queue: VecDeque<ConnectToPeerResponse>,
    queue: VecDeque<ConnectToPeerResponse>,
}

#[derive(Debug)]
struct UserState {
    username: String,
    password: String,
    pass_hash: String,
    ipv4: Ipv4Addr,
}

impl Default for UserState {
    fn default() -> Self {
        Self {
            username: "".into(),
            password: "".into(),
            pass_hash: "".into(),
            ipv4: Ipv4Addr::from(0),
        }
    }
}

#[derive(Default, Debug)]
struct SearchState {
    search_query: String,
    searches: HashMap<u32, Vec<FileSearchResponsePeer>>,
}

#[derive(Default, Debug)]
struct Reverie {
    server_worker: Option<mpsc::Sender<ServerCommand>>,
    peer_worker: Option<mpsc::Sender<PeerCommand>>,

    server_state: ServerState,
    peer_state: PeerState,
    user_state: UserState,
    search_state: SearchState,
}

impl Reverie {
    fn new() -> (Self, Task<ReverieMessage>) {
        (Self::default(), Task::none())
    }

    fn theme(&self) -> iced::Theme {
        iced::Theme::GruvboxDark
    }

    fn update(&mut self, message: ReverieMessage) -> Task<ReverieMessage> {
        use PeerEvent as PE;
        use ReverieMessage as RM;
        use ServerEvent as SE;

        match message {
            RM::Periodic => {
                if self.peer_state.active_since.is_some()
                    && self.peer_state.active_since.unwrap().elapsed() > MAX_PEER_TTL
                {
                    if let Some(active_peer) = self.peer_state.active_peer.clone() {
                        self.peer_state.queue.push_back(active_peer);
                    }
                    Task::done(RM::NextPeer)
                } else {
                    Task::none()
                }
            }
            RM::NextPeer => {
                let peer = self.peer_state.queue.pop_front();
                let mut peer_worker = self.peer_worker.clone();

                if peer.is_none() || peer_worker.is_none() {
                    Task::none()
                } else {
                    self.peer_state.active_peer = Some(peer.clone().unwrap());
                    self.peer_state.active_since = Some(Instant::now());
                    self.peer_state.expected_token = peer.clone().unwrap().token;
                    Task::perform(
                        async move {
                            let peer_worker = peer_worker.as_mut().unwrap();
                            let peer = peer.as_ref().unwrap();
                            let _ = peer_worker.send(PeerCommand::Reset).await;
                            let _ = peer_worker.send(PeerCommand::Init(peer.address)).await;
                            let _ = peer_worker
                                .send(PeerCommand::PierceFirewall(peer.token))
                                .await;
                        },
                        |_| RM::SentCommand,
                    )
                }
            }
            RM::Peer(s) => match s {
                PE::Ready(conn) => {
                    println!("PEER WORKER: Ready!");
                    self.peer_worker = Some(conn);
                    Task::none()
                }
                PE::InitSuccess => {
                    println!("PEER WORKER: Successfully initialized connection");
                    Task::none()
                }
                PE::InitFailed => {
                    println!("PEER WORKER: Failed to initialize connection");
                    Task::done(RM::NextPeer)
                }
                PE::PierceFirewall(resp) => {
                    if resp.token == self.peer_state.expected_token {
                        println!("PEER WORKER: Successfully pierced firewall");
                        Task::none()
                    } else {
                        println!("PEER WORKER: Failed to pierce firewall");
                        Task::done(RM::NextPeer)
                    }
                }
                PE::FileSearch(resp) => {
                    if let Some(searches) = self.search_state.searches.get_mut(&resp.token) {
                        searches.push(resp);
                    }
                    Task::none()
                }
                _ => Task::none(),
            },
            RM::Server(s) => match s {
                SE::Connected(conn) => {
                    self.server_worker = Some(conn);
                    self.server_state.connected = true;
                    Task::none()
                }
                SE::LoginSuccess(resp) => {
                    self.server_state.logged_in = true;
                    self.server_state.latest_login_failure = None;
                    self.user_state.pass_hash = resp.hash;
                    self.user_state.ipv4 = resp.ipv4;
                    if let Some(mut slsk) = self.server_worker.clone() {
                        Task::perform(
                            async move {
                                slsk.send(ServerCommand::SetWaitPort {
                                    port: LISTENER_PORT,
                                    obfuscated_port: OBFUSCATED_PORT,
                                })
                                .await
                            },
                            |_| RM::SentCommand,
                        )
                    } else {
                        Task::none()
                    }
                }
                SE::LoginFailure(resp) => {
                    self.server_state.logged_in = false;
                    self.server_state.latest_login_failure = Some(resp.reason);
                    if let Some(mut slsk) = self.server_worker.clone() {
                        Task::perform(
                            async move { slsk.send(ServerCommand::ResetConnection).await },
                            |_| RM::SentCommand,
                        )
                    } else {
                        Task::none()
                    }
                }
                SE::ConnectToPeer(resp) => {
                    if resp.privileged {
                        self.peer_state.privileged_queue.push_back(resp);
                    } else {
                        self.peer_state.queue.push_back(resp);
                    }
                    if self.peer_state.active_peer.is_none() {
                        Task::done(RM::NextPeer)
                    } else {
                        Task::none()
                    }
                }
                _ => Task::none(),
            },
            RM::LoginPressed => {
                if let Some(mut slsk) = self.server_worker.clone() {
                    let username = self.user_state.username.clone();
                    let password = self.user_state.password.clone();
                    Task::perform(
                        async move { slsk.send(ServerCommand::Login { username, password }).await },
                        |_| RM::SentCommand,
                    )
                } else {
                    Task::none()
                }
            }
            RM::UsernameInputChange(content) => {
                self.user_state.username = content;
                Task::none()
            }
            RM::PasswordInputChange(content) => {
                self.user_state.password = content;
                Task::none()
            }
            RM::SearchQueryChange(content) => {
                self.search_state.search_query = content;
                Task::none()
            }
            RM::SearchPressed => {
                let _ = self.search_state.searches.insert(123, vec![]);
                if let Some(mut slsk) = self.server_worker.clone() {
                    let search_query = self.search_state.search_query.clone();
                    Task::perform(
                        async move {
                            slsk.send(ServerCommand::FileSearch {
                                token: 123,
                                search_query,
                            })
                            .await
                        },
                        |_| RM::SentCommand,
                    )
                } else {
                    Task::none()
                }
            }
            _ => Task::none(),
        }
    }

    fn subscription(&self) -> Subscription<ReverieMessage> {
        let periodic = iced::time::every(Duration::from_millis(50)).map(|_| ReverieMessage::Periodic);
        let server = Subscription::run(server_worker).map(ReverieMessage::Server);
        let peer = Subscription::run(peer_worker).map(ReverieMessage::Peer);
        Subscription::batch(vec![server, peer, periodic].into_iter())
    }

    fn view(&self) -> Element<ReverieMessage> {
        let logged_in: &str = if self.server_state.logged_in {
            "Logged in"
        } else {
            "Logged out"
        };
        let login_error = if let Some(failure_reason) = self.server_state.latest_login_failure {
            match failure_reason {
                LoginFailureReason::InvalidPass => "Invalid password",
                LoginFailureReason::InvalidUsername => "Invalid username",
                LoginFailureReason::InvalidVersion => "Invalid Reverie version",
            }
        } else {
            ""
        };

        let mut search_results: Vec<FileSearchResult> = vec![];
        if let Some(searches) = self.search_state.searches.get(&123) {
            for resp in searches {
                search_results.append(&mut resp.results.clone());
            }
        }

        container(column! {
            row! {
                text_input("Username", &self.user_state.username)
                    .on_input(ReverieMessage::UsernameInputChange),
                text_input("Password", &self.user_state.password)
                    .on_input(ReverieMessage::PasswordInputChange),
            },
            button("log in").on_press(ReverieMessage::LoginPressed),
            text(logged_in).size(15),
            text(login_error).size(15).color(Color::new(1.0, 0.0, 0.0, 1.0)),

            text_input("Search", &self.search_state.search_query)
                .on_input(ReverieMessage::SearchQueryChange),
            button("Search").on_press(ReverieMessage::SearchPressed),
            text(format!("Number of queued connections: {}", self.peer_state.queue.len() + self.peer_state.privileged_queue.len())).size(15),

            scrollable(column(search_results.into_iter().map(|res: FileSearchResult| text(res.name).size(10)).map(Element::from))),
        })
        .padding(10)
        .center_x(Fill)
        .center_y(Fill)
        .into()
    }
}

fn main() -> Result<()> {
    iced::application("Reverie", Reverie::update, Reverie::view)
        .subscription(Reverie::subscription)
        .theme(Reverie::theme)
        .run_with(Reverie::new)?;
    Ok(())
}
