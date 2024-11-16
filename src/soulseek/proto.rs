use super::packing::{ReadSoulSeekBytes, WriteSoulSeekBytes};
use crate::error::{Error, Result};
use flate2::bufread::ZlibDecoder;
use std::{
    io::{Cursor, Seek},
    net::{Ipv4Addr, SocketAddrV4},
};

pub fn login_request(username: String, password: String) -> Result<Vec<u8>> {
    let mut pack = Cursor::new(Vec::new());

    let hash = format!(
        "{:x}",
        md5::compute(&[username.as_bytes(), password.as_bytes()].concat())
    );

    // Reserve space for length
    pack.pack_u32(0)?;
    // Set code
    pack.pack_u32(1)?;

    // Data
    pack.pack_string(username)?;
    pack.pack_string(password)?;
    pack.pack_u32(160)?;
    pack.pack_string(hash)?;
    pack.pack_u32(1)?;

    // Set the length
    pack.rewind()?;
    pack.pack_u32(pack.get_ref().len() as u32 - 4)?;

    Ok(pack.into_inner())
}

#[derive(Debug, Clone)]
pub struct LoginResponseSuccess {
    pub greet: String,
    pub ipv4: Ipv4Addr,
    pub hash: String,
    pub is_supporter: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum LoginFailureReason {
    InvalidPass,
    InvalidUsername,
    InvalidVersion,
}

impl TryFrom<String> for LoginFailureReason {
    type Error = Error;

    fn try_from(s: String) -> core::result::Result<Self, Self::Error> {
        match s.as_str() {
            "INVALIDPASS" => Ok(LoginFailureReason::InvalidPass),
            "INVALIDUSERNAME" => Ok(LoginFailureReason::InvalidUsername),
            "INVALIDVERSION" => Ok(LoginFailureReason::InvalidVersion),
            _ => Err(Self::Error::InvalidFailureReason),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoginResponseFailure {
    pub reason: LoginFailureReason,
}

#[derive(Debug, Clone)]
pub enum LoginResponse {
    Success(LoginResponseSuccess),
    Failure(LoginResponseFailure),
}

impl LoginResponse {
    pub fn from_bytes(buf: &[u8]) -> Result<Self> {
        let mut buf = buf;

        if buf.unpack_bool()? {
            let greet = buf.unpack_string()?;
            let ipv4 = buf.unpack_ipv4()?;
            let hash = buf.unpack_string()?;
            let is_supporter = buf.unpack_bool()?;

            Ok(Self::Success(LoginResponseSuccess {
                greet,
                ipv4,
                hash,
                is_supporter,
            }))
        } else {
            let reason = buf.unpack_string()?.try_into()?;
            Ok(Self::Failure(LoginResponseFailure { reason }))
        }
    }
}

#[derive(Debug, Clone, derive_more::Display)]
pub enum ConnectionType {
    /// Peer to Peer
    #[display("P")]
    PeerToPeer,

    /// File Transfer
    #[display("F")]
    FileTransfer,

    /// Distributed Network
    #[display("D")]
    DistributedNetwork,
}

impl TryFrom<String> for ConnectionType {
    type Error = Error;

    fn try_from(s: String) -> core::result::Result<Self, Self::Error> {
        match s.as_str() {
            "P" => Ok(Self::PeerToPeer),
            "F" => Ok(Self::FileTransfer),
            "D" => Ok(Self::DistributedNetwork),
            _ => Err(Self::Error::InvalidConnectionType),
        }
    }
}

pub fn set_wait_port_request(port: u16, obfuscated_port: u16) -> Result<Vec<u8>> {
    let mut request = Cursor::new(Vec::new());

    // Reserve space for the length
    request.pack_u32(0)?;

    // Set the code, 18 in the case of ConnectToPeer
    request.pack_u32(2)?;

    // Request data
    request.pack_u32(port as u32)?;
    request.pack_u32(1)?;
    request.pack_u32(obfuscated_port as u32)?;

    // Set the length
    request.rewind()?;
    request.pack_u32(request.get_ref().len() as u32 - 4)?;

    Ok(request.into_inner())
}

pub fn connect_to_peer_request(
    token: u32,
    username: String,
    conn_type: ConnectionType,
) -> Result<Vec<u8>> {
    let mut request = Cursor::new(Vec::new());

    // Reserve space for the length
    request.pack_u32(0)?;

    // Set the code, 18 in the case of ConnectToPeer
    request.pack_u32(18)?;

    // Request data
    request.pack_u32(token)?;
    request.pack_string(username)?;
    request.pack_string(conn_type.to_string())?;

    // Set the length
    request.rewind()?;
    request.pack_u32(request.get_ref().len() as u32 - 4)?;

    Ok(request.into_inner())
}

#[derive(Debug, Clone)]
pub struct ConnectToPeerResponse {
    pub username: String,
    pub conn_type: ConnectionType,
    pub address: std::net::SocketAddrV4,
    pub token: u32,
    pub privileged: bool,
    pub obfuscated_port: u32,
}

impl ConnectToPeerResponse {
    pub fn from_bytes(buf: &[u8]) -> Result<Self> {
        let mut buf = buf;
        let resp = Self {
            username: buf.unpack_string()?,
            conn_type: buf.unpack_string()?.try_into()?,
            address: SocketAddrV4::new(buf.unpack_ipv4()?, buf.unpack_u32()? as u16),
            token: buf.unpack_u32()?,
            privileged: buf.unpack_bool()?,
            obfuscated_port: buf.unpack_u32()?,
        };
        Ok(resp)
    }
}

#[derive(Debug, Clone)]
pub struct PrivilegedUsersResponse(Vec<String>);

impl PrivilegedUsersResponse {
    pub fn from_bytes(buf: &[u8]) -> Result<Self> {
        let mut buf = buf;
        let len = buf.unpack_u32()?;
        let privileged_users = (0..len)
            .into_iter()
            .map(|_| -> Result<String> { Ok(buf.unpack_string()?) })
            .collect::<Result<Vec<String>>>()?;
        Ok(PrivilegedUsersResponse(privileged_users))
    }
}

#[derive(Debug, Clone)]
pub struct ExcludedSearchPhrasesResponse(Vec<String>);

impl ExcludedSearchPhrasesResponse {
    pub fn from_bytes(buf: &[u8]) -> Result<Self> {
        let mut buf = buf;
        let len = buf.unpack_u32()?;
        let excluded_phrases = (0..len)
            .into_iter()
            .map(|_| -> Result<String> { Ok(buf.unpack_string()?) })
            .collect::<Result<Vec<String>>>()?;
        Ok(ExcludedSearchPhrasesResponse(excluded_phrases))
    }
}

pub fn file_search_request(token: u32, query: String) -> Result<Vec<u8>> {
    let mut request = Cursor::new(Vec::new());
    // Reserve space for the length
    request.pack_u32(0)?;

    // Set the code, 18 in the case of ConnectToPeer
    request.pack_u32(26)?;

    request.pack_u32(token)?;
    request.pack_string(query)?;

    // Set the length
    request.rewind()?;
    request.pack_u32(request.get_ref().len() as u32 - 4)?;

    Ok(request.into_inner())
}

#[derive(Debug, Clone)]

pub struct FileSearchResponse {
    username: String,
    token: u32,
    search_query: String,
}

impl FileSearchResponse {
    pub fn from_bytes(buf: &[u8]) -> Result<Self> {
        let mut buf = buf;
        let resp = Self {
            username: buf.unpack_string()?,
            token: buf.unpack_u32()?,
            search_query: buf.unpack_string()?,
        };
        Ok(resp)
    }
}

pub fn pierce_firewall_request(token: u32) -> Result<Vec<u8>> {
    let mut request = Cursor::new(Vec::new());
    // Reserve space for the length
    request.pack_u32(0)?;

    // Same as packing a 0 uint8 for the PierceFirewall code
    request.pack_bool(false)?;

    request.pack_u32(token)?;

    // Set the length
    request.rewind()?;
    request.pack_u32(request.get_ref().len() as u32 - 4)?;

    Ok(request.into_inner())
}

#[derive(Debug, Clone)]
pub struct PierceFirewallResponse {
    pub token: u32,
}

impl PierceFirewallResponse {
    pub fn from_bytes(buf: &[u8]) -> Result<Self> {
        let mut buf = buf;
        let token = buf.unpack_u32()?;
        Ok(Self { token })
    }
}

#[derive(Debug, Clone)]
pub enum FileAttribute {
    Bitrate(u32),
    Duration(u32),
    VBR(bool),
    Encoder,
    SampleRate(u32),
    BitDepth(u32),
}

impl TryFrom<(u32, u32)> for FileAttribute {
    type Error = Error;

    fn try_from(pair: (u32, u32)) -> core::result::Result<Self, Self::Error> {
        let (code, value) = pair;
        match code {
            0 => Ok(Self::Bitrate(value)),
            1 => Ok(Self::Duration(value)),
            2 => Ok(Self::VBR(value == 1)),
            3 => Ok(Self::Encoder),
            4 => Ok(Self::SampleRate(value)),
            5 => Ok(Self::BitDepth(value)),
            _ => Err(Error::InvalidFileAttributeCode),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileSearchResult {
    pub name: String,
    pub size: u64,
    pub extension: String,
    pub attributes: Vec<FileAttribute>,
}

#[derive(Debug, Clone)]
pub struct FileSearchResponsePeer {
    pub username: String,
    pub token: u32,
    pub results: Vec<FileSearchResult>,
    pub avg_speed: u32,
    pub queue_length: u32,
    pub private_results: Vec<FileSearchResult>,
}

impl FileSearchResponsePeer {
    pub fn from_bytes(buf: &[u8]) -> Result<Self> {
        let mut buf = ZlibDecoder::new(buf);
        println!("buf: {:02x?}", buf);
        let username = buf.unpack_string().unwrap();
        let token = buf.unpack_u32().unwrap();
        let results = (0..buf.unpack_u32().unwrap())
            .map(|_| -> Result<FileSearchResult> {
                let _ = buf.unpack_bool().unwrap();
                let result = FileSearchResult {
                    name: buf.unpack_string().unwrap(),
                    size: buf.unpack_u64().unwrap(),
                    extension: buf.unpack_string().unwrap(),
                    attributes: (0..buf.unpack_u32().unwrap())
                        .map(|_| -> Result<FileAttribute> {
                            (buf.unpack_u32().unwrap(), buf.unpack_u32().unwrap()).try_into()
                        })
                        .collect::<Result<Vec<FileAttribute>>>()
                        .unwrap(),
                };
                Ok(result)
            })
            .collect::<Result<Vec<FileSearchResult>>>()
            .unwrap();

        // slotfree
        let _ = buf.unpack_bool().unwrap();

        let avg_speed = buf.unpack_u32().unwrap();
        let queue_length = buf.unpack_u32().unwrap();
        let _ = buf.unpack_u32().unwrap();

        let private_results = (0..buf.unpack_u32().unwrap())
            .map(|_| -> Result<FileSearchResult> {
                let _ = buf.unpack_bool()?;
                let result = FileSearchResult {
                    name: buf.unpack_string().unwrap(),
                    size: buf.unpack_u64().unwrap(),
                    extension: buf.unpack_string().unwrap(),
                    attributes: (0..buf.unpack_u32().unwrap())
                        .map(|_| -> Result<FileAttribute> {
                            (buf.unpack_u32().unwrap(), buf.unpack_u32().unwrap()).try_into()
                        })
                        .collect::<Result<Vec<FileAttribute>>>()
                        .unwrap(),
                };
                Ok(result)
            })
            .collect::<Result<Vec<FileSearchResult>>>()
            .unwrap();

        Ok(Self {
            username,
            token,
            results,
            avg_speed,
            queue_length,
            private_results,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_login() -> Result<()> {
        let req = login_request("username".into(), "password".into())?;
        Ok(())
    }
}
