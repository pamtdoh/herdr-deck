//! Wire protocol between a host bridge and the pixbar: one JSON object per line over TCP.
//!
//! The device listens; any number of hosts connect (WiFi directly, or over the USB cable through the panel's adbd).
//! Hosts push their agent list, the device pushes what the user asked for. Agents are addressed by the
//! host's own id (the herdr pane id), never by list position, so a list that changed under a press is harmless.

use pixbar_render::{Agent, Effort, Model};
use serde::{Deserialize, Serialize};

pub const DEFAULT_PORT: u16 = 17002;
/// Said by both sides in their Hello. What one side does not know of the other's messages it skips (an unknown
/// status reads as unknown, an agent that does not parse is left out, an unknown message is logged), so the
/// number is for telling a user why something is missing, not a gate.
/// 2: models are names, agents carry `reported` / `has_effort` / `next_model`, the beacon carries the MAC.
pub const PROTO: u32 = 2;
/// The device announces `PIXBAR <tcp-port> <mac>` here once a second; it has no mDNS and no resolver.
pub const BEACON_PORT: u16 = 17003;
pub const BEACON_PREFIX: &str = "PIXBAR";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentState {
    pub id: String,
    #[serde(flatten)]
    pub agent: Agent,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToDevice {
    Hello {
        host: String,
        #[serde(default)]
        proto: u32,
    },
    /// Full replacement of this host's agents, in herdr sidebar order. `focused` is an agent id.
    State { agents: Vec<AgentState>, focused: Option<String> },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    Focus,
    SetEffort { effort: Effort },
    SetModel { model: Model },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FromDevice {
    /// `build` tells one build of the device program from another (a hash of its executable).
    /// `mac`: which panel this is, the same one its stock firmware announces.
    Hello {
        version: String,
        #[serde(default)]
        build: String,
        #[serde(default)]
        mac: String,
        /// The kernel's boot id: tells one power-up of the panel from the next.
        #[serde(default)]
        boot: String,
        #[serde(default)]
        proto: u32,
    },
    /// STOCK FW was picked on the panel: Ulanzi's app takes over now, and no host is to start the program
    /// again by itself.
    Stock,
    /// Panel settings that are the host's to carry out. `refresh_ms`: how often to re-read what herdr sends
    /// no event for (model, effort, context). Sent on connect and whenever it changes.
    Config { refresh_ms: u32 },
    Intent {
        id: String,
        #[serde(flatten)]
        action: Action,
    },
}

pub fn encode<T: Serialize>(msg: &T) -> String {
    let mut line = serde_json::to_string(msg).expect("protocol types always serialise");
    line.push('\n');
    line
}

pub fn decode<'a, T: Deserialize<'a>>(line: &'a str) -> Result<T, serde_json::Error> {
    serde_json::from_str(line.trim())
}

/// What a host sent, as far as this build understands it: an agent that does not parse (a newer host, a value
/// this build has no word for) is left out rather than taking the whole list with it.
pub fn decode_to_device(line: &str) -> Result<ToDevice, serde_json::Error> {
    decode::<ToDevice>(line).or_else(|strict| {
        let mut v: serde_json::Value = serde_json::from_str(line.trim())?;
        if v["type"] != "state" {
            return Err(strict);
        }
        let agents = v["agents"].as_array_mut().map(std::mem::take).unwrap_or_default();
        let agents = agents.into_iter().filter_map(|a| serde_json::from_value(a).ok()).collect();
        Ok(ToDevice::State { agents, focused: v["focused"].as_str().map(str::to_string) })
    })
}

/// Accumulates socket reads and hands back complete lines.
#[derive(Default)]
pub struct LineBuffer {
    buf: Vec<u8>,
}

impl LineBuffer {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    pub fn next_line(&mut self) -> Option<String> {
        let end = self.buf.iter().position(|&b| b == b'\n')?;
        let line: Vec<u8> = self.buf.drain(..=end).collect();
        Some(String::from_utf8_lossy(&line).trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pixbar_render::{Limit, Status};

    #[test]
    fn messages_round_trip_as_single_lines() {
        let state = ToDevice::State {
            agents: vec![AgentState {
                id: "w7:p1".into(),
                agent: Agent {
                    space: "web".into(),
                    tab: "2".into(),
                    reported: true,
                    has_effort: true,
                    next_model: Some(Model::new("opus")),
                    status: Status::Working,
                    model: Model::new("fable"),
                    effort: Effort::Ultra,
                    ctx_used: 104_000,
                    ctx_window: 1_000_000,
                    ultra_ok: true,
                    dir: "web".into(),
                    title: "Fix the parser".into(),
                    fresh: false,
                    session: "parser".into(),
                    cost_cents: 1_234,
                    limit_5h: Some(Limit { used_pct: 12, resets_in_min: 200 }),
                    limit_7d: None,
                },
            }],
            focused: Some("w7:p1".into()),
        };
        let line = encode(&state);
        assert_eq!(line.matches('\n').count(), 1);
        assert_eq!(decode::<ToDevice>(&line).unwrap(), state);

        let intent = FromDevice::Intent { id: "w7:p1".into(), action: Action::SetEffort { effort: Effort::Max } };
        let line = encode(&intent);
        assert_eq!(line.trim(), r#"{"type":"intent","id":"w7:p1","action":"set_effort","effort":"max"}"#);
        assert_eq!(decode::<FromDevice>(&line).unwrap(), intent);
    }

    #[test]
    fn what_this_build_does_not_know_costs_one_agent_not_the_list() {
        let agent = |id: &str, effort: &str, status: &str| {
            format!(r#"{{"id":"{id}","space":"s","tab":"1","status":"{status}","model":"sonnet","effort":"{effort}","ctx_used":1,"ctx_window":2,"ultra_ok":false}}"#)
        };
        let line = format!(r#"{{"type":"state","agents":[{},{},{}],"focused":"a","later":1}}"#, agent("a", "high", "working"), agent("b", "turbo", "idle"), agent("c", "low", "napping"));
        let ToDevice::State { agents, focused } = decode_to_device(&line).unwrap() else { panic!("a state") };
        assert_eq!(agents.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["a", "c"], "an effort level nobody has heard of");
        assert_eq!((agents[1].agent.status, agents[0].agent.model.word(), focused.as_deref()), (Status::Unknown, "SONNET", Some("a")));
        // An older host says nothing about these: the session counts as reported, with effort levels, nothing to switch to.
        assert_eq!((agents[0].agent.reported, agents[0].agent.has_effort, agents[0].agent.next_model), (true, true, None));
        assert!(decode_to_device(r#"{"type":"bogus"}"#).is_err());
    }

    #[test]
    fn line_buffer_survives_split_reads() {
        let mut lb = LineBuffer::default();
        lb.push(b"{\"a\":1}\n{\"b\"");
        assert_eq!(lb.next_line().as_deref(), Some("{\"a\":1}"));
        assert_eq!(lb.next_line(), None);
        lb.push(b":2}\n");
        assert_eq!(lb.next_line().as_deref(), Some("{\"b\":2}"));
    }
}
