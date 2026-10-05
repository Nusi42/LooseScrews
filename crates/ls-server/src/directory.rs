//! The server list entry (plan §3.1, M3 package B7): with `--directory
//! <url>` the server registers with an `ls-directory` service and sends a
//! heartbeat every 10 s ([`ls_directory::spawn_heartbeats`]) describing
//! itself ([`server_info`]): `--server-name`, `--region`, the public
//! address players reach it at (`--public-address`), the WebTransport
//! certificate hash, the public room's mode, world and rule set code, the
//! player count and the roster of present players (for duel challenges).
//! On shutdown it removes its entry. A server without `--directory` runs
//! unchanged and stays reachable by link or address.
//!
//! The directory also relays duel challenges to players on other servers
//! ([`crate::duel`], [`crate::api`]).

use std::net::SocketAddr;

use ls_directory::api::{data_hash_hex, RosterEntry, ServerInfo, MAX_NAME, MAX_ROSTER};

use crate::rooms::RoomStatus;
use crate::server::Listing;

/// `--server-name` default.
pub const DEFAULT_SERVER_NAME: &str = "Loose Screws server";

/// The public `host:port` of a server: `--public-address`, else the bound
/// address (`localhost:<port>` for the unspecified "every interface"
/// address, which only suits local play: hosts on a network pass
/// `--public-address`).
pub fn public_address(listing: &Listing, local: SocketAddr) -> String {
    if let Some(a) = listing
        .public_address
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty())
    {
        return a.to_string();
    }
    if local.ip().is_unspecified() {
        if listing.directory.is_some() {
            tracing::warn!(
                "--directory without --public-address on an unspecified bind address: \
                 announcing localhost:{}",
                local.port()
            );
        }
        format!("localhost:{}", local.port())
    } else {
        local.to_string()
    }
}

/// What the server tells the directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfoInput<'a> {
    pub listing: &'a Listing,
    pub address: &'a str,
    pub cert_hash: Option<String>,
    /// The public room's status.
    pub public: &'a RoomStatus,
    /// Players over all rooms.
    pub players: u64,
    pub data_hash: u64,
    /// Present players (public id, name).
    pub roster: &'a [(String, String)],
}

/// The directory entry for this server.
pub fn server_info(i: &InfoInput) -> ServerInfo {
    let name = i
        .listing
        .server_name
        .as_deref()
        .and_then(crate::duel::clean_name)
        .unwrap_or_else(|| DEFAULT_SERVER_NAME.to_string());
    let name: String = name.chars().take(MAX_NAME).collect();
    let region: String = i
        .listing
        .region
        .chars()
        .filter(|c| !c.is_control())
        .take(ls_directory::api::MAX_REGION)
        .collect();
    let max_players = u16::from(i.public.max_players).max(1);
    ServerInfo {
        name,
        address: i.address.to_string(),
        cert_hash: i.cert_hash.clone(),
        region,
        mode: i.public.mode.to_string(),
        map_code: i.public.share_code.clone(),
        rules_code: (!i.public.rules_code.is_empty()).then(|| i.public.rules_code.clone()),
        players: (i.players.min(u64::from(max_players))) as u16,
        max_players,
        protocol: ls_net::PROTOCOL_VERSION,
        data_hash: data_hash_hex(i.data_hash),
        roster: i
            .roster
            .iter()
            .take(MAX_ROSTER)
            .map(|(id, name)| RosterEntry {
                name: name.clone(),
                id: id.clone(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_address_defaults() {
        let mut l = Listing::default();
        let local: SocketAddr = "127.0.0.1:7777".parse().unwrap();
        assert_eq!(public_address(&l, local), "127.0.0.1:7777");
        let any = SocketAddr::new(std::net::Ipv4Addr::new(0, 0, 0, 0).into(), 7000);
        assert_eq!(public_address(&l, any), "localhost:7000");
        l.public_address = Some(" game.example.com:443 ".into());
        assert_eq!(public_address(&l, any), "game.example.com:443");
    }

    #[test]
    fn server_info_is_valid_for_the_directory() {
        let listing = Listing {
            directory: Some("http://127.0.0.1:7780".into()),
            server_name: Some("x".repeat(80)),
            region: "eu-west".into(),
            public_address: None,
        };
        let status = RoomStatus {
            label: "public".into(),
            phase: "match",
            mode: "team_brawl",
            entry: "team_brawl".into(),
            share_code: "LS2-ABCDEF".into(),
            ruleset: "Team Brawl".into(),
            rules_code: String::new(),
            players: 3,
            max_players: 8,
            closed: false,
        };
        let roster = vec![("player-0001".to_string(), "Ace".to_string())];
        let info = server_info(&InfoInput {
            listing: &listing,
            address: "127.0.0.1:7777",
            cert_hash: Some("ab".repeat(32)),
            public: &status,
            players: 5,
            data_hash: 0x0123_4567_89ab_cdef,
            roster: &roster,
        });
        info.validate().unwrap();
        assert_eq!(info.name.chars().count(), MAX_NAME);
        assert_eq!(info.mode, "team_brawl");
        assert_eq!(info.players, 5);
        assert_eq!(info.max_players, 8);
        assert_eq!(info.rules_code, None);
        assert_eq!(info.roster[0].name, "Ace");
        assert_eq!(info.protocol, ls_net::PROTOCOL_VERSION);
        let unnamed = Listing::default();
        let info = server_info(&InfoInput {
            listing: &unnamed,
            players: 99,
            ..InfoInput {
                listing: &listing,
                address: "h:1",
                cert_hash: None,
                public: &status,
                players: 0,
                data_hash: 1,
                roster: &[],
            }
        });
        assert_eq!(info.name, DEFAULT_SERVER_NAME);
        assert_eq!(info.players, 8, "capped at max_players");
        info.validate().unwrap();
    }
}
