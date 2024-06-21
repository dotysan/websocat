use std::net::SocketAddr;

use super::types::{Endpoint, Overlay, SpecifierStack};

impl SpecifierStack {
    pub fn from_str(mut x: &str) -> anyhow::Result<SpecifierStack> {
        let innermost;
        let mut overlays = vec![];

        loop {
            match ParseStrChunkResult::from_str(x)? {
                ParseStrChunkResult::Endpoint(e) => {
                    innermost = e;
                    break;
                }
                ParseStrChunkResult::Overlay { ovl, rest } => {
                    overlays.push(ovl);
                    x = rest;
                }
            }
        }

        overlays.reverse();

        Ok(SpecifierStack {
            innermost: innermost,
            overlays,
        })
    }
}

enum ParseStrChunkResult<'a> {
    Endpoint(Endpoint),
    Overlay { ovl: Overlay, rest: &'a str },
}

impl ParseStrChunkResult<'_> {
    fn from_str(x: &str) -> anyhow::Result<ParseStrChunkResult<'_>> {
        if x.starts_with("ws://") {
            let u = http::Uri::try_from(x)?;
            if u.authority().is_none() {
                anyhow::bail!("ws:// URL without authority");
            }
            Ok(ParseStrChunkResult::Endpoint(Endpoint::WsUrl(u)))
        } else if x.starts_with("wss://") {
            let u = http::Uri::try_from(x)?;
            if u.authority().is_none() {
                anyhow::bail!("wss:// URL without authority");
            }
            Ok(ParseStrChunkResult::Endpoint(Endpoint::WssUrl(u)))
        } else if let Some(rest) = x.strip_prefix("tcp:") {
            let a: Result<SocketAddr, _> = rest.parse();
            match a {
                Ok(a) => Ok(ParseStrChunkResult::Endpoint(Endpoint::TcpConnectByIp(a))),
                Err(_) => Ok(ParseStrChunkResult::Endpoint(
                    Endpoint::TcpConnectByLateHostname {
                        hostname: rest.to_owned(),
                    },
                )),
            }
        } else if let Some(rest) = x.strip_prefix("tcp-l:") {
            let a: SocketAddr = rest.parse()?;
            Ok(ParseStrChunkResult::Endpoint(Endpoint::TcpListen(a)))
        } else if x == "-" || x == "stdio:" {
            Ok(ParseStrChunkResult::Endpoint(Endpoint::Stdio))
        } else if let Some(rest) = x.strip_prefix("tls:") {
            Ok(ParseStrChunkResult::Overlay {
                ovl: Overlay::TlsClient {
                    domain: String::new(),
                    varname_for_connector: String::new(),
                },
                rest,
            })
        } else if let Some(rest) = x.strip_prefix("ws-accept:") {
            Ok(ParseStrChunkResult::Overlay {
                ovl: Overlay::WsAccept {  },
                rest,
            })
        } else if let Some(rest) = x.strip_prefix("ws-ll-client:") {
            Ok(ParseStrChunkResult::Overlay {
                ovl: Overlay::WsFramer { client_mode: true },
                rest,
            })
        } else if let Some(rest) = x.strip_prefix("ws-ll-server:") {
            Ok(ParseStrChunkResult::Overlay {
                ovl: Overlay::WsFramer { client_mode: false },
                rest,
            })
        } else if let Some(rest) = x.strip_prefix("ws-l:") {
            let a: SocketAddr = rest.parse()?;
            Ok(ParseStrChunkResult::Endpoint(Endpoint::WsListen(a)))
        } else {
            anyhow::bail!("Unknown specifier: {x}")
        }
    }
}
