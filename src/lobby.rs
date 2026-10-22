use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tracing::{info, warn};

use crate::protocol::{
    ClientToServerPacket, FollowerState, PlayerState, ServerToClientPacket, AUTO_PLAYER_ID,
    COOP_MAGIC, MAX_PLAYERS, PROTOCOL_VERSION, REMOTE_SLOTS,
};

const MAX_NPCS_BEFORE_FOLLOWER_CULL: u8 = 26;

#[derive(Debug, Clone)]
pub struct ConnectedPlayer {
    pub slot_id: u8,
    pub addr: SocketAddr,
    pub last_seen: Instant,
    pub last_sequence: u32,
    pub heartbeat_rom: u32,
    pub map_npc_count: u8,
    pub state: PlayerState,
    pub lead_species: u16,
    pub lead_form: u16,
    pub lead_is_shiny: u8,
    pub lead_gender: u8,
    pub badge_mask: u16,
    pub assigned_monolocke: u8,
}

pub struct LobbyState {
    slots: [Option<ConnectedPlayer>; MAX_PLAYERS],
    client_timeout: Duration,
    min_players_for_sync: usize,
    server_tick: u32,
    inspect_all_packets: bool,
}

impl LobbyState {
    pub fn new(client_timeout: Duration, min_players_for_sync: usize, inspect_all_packets: bool) -> Self {
        Self {
            slots: Default::default(),
            client_timeout,
            min_players_for_sync: min_players_for_sync.clamp(1, MAX_PLAYERS),
            server_tick: 0,
            inspect_all_packets,
        }
    }

    /// Processes an incoming UDP update from a BizHawk client.
    pub fn handle_client_packet(&mut self, addr: SocketAddr, pkt: ClientToServerPacket) -> Option<u8> {
        let now = Instant::now();
        let slot_idx = self.resolve_or_assign_slot(addr, pkt.requested_player_id)?;

        if let Some(existing) = &mut self.slots[slot_idx] {
            // Guard against out-of-order UDP datagrams (allowing sequence reset if emulator restarts)
            if pkt.sequence < existing.last_sequence && existing.last_sequence.wrapping_sub(pkt.sequence) < 10_000 {
                return Some(slot_idx as u8);
            }

            // Emit human-readable inspection log whenever meaningful game state changes
            let old = &existing.state;
            let new = &pkt.local_player;
            let state_changed = old.map_id != new.map_id
                || old.x != new.x
                || old.y != new.y
                || old.direction != new.direction
                || existing.lead_species != pkt.lead_species
                || existing.badge_mask != pkt.badge_mask_local;

            if state_changed || self.inspect_all_packets {
                info!(
                    "[P{slot_idx} @ {addr}] Map: {} | Pos: ({}, {}) [Prev: ({}, {})] | Dir: {} | Follower: #{} (Shiny: {}) | Badges: 0b{:08b} | ROM_HB: {}",
                    new.map_id,
                    new.x,
                    new.y,
                    new.prev_x,
                    new.prev_y,
                     Self::dir_name(new.direction),
                    pkt.lead_species,
                    pkt.lead_is_shiny,
                    pkt.badge_mask_local,
                    pkt.heartbeat_rom
                );
            }

            // Preserve previous tile if client sent identical x/y and prev_x/prev_y
            let mut updated_state = pkt.local_player;
            updated_state.is_active = 1;
            if (updated_state.x != old.x || updated_state.y != old.y)
                && updated_state.prev_x == updated_state.x
                && updated_state.prev_y == updated_state.y
            {
                updated_state.prev_x = old.x;
                updated_state.prev_y = old.y;
            }

            existing.addr = addr;
            existing.last_seen = now;
            existing.last_sequence = pkt.sequence;
            existing.heartbeat_rom = pkt.heartbeat_rom;
            existing.map_npc_count = pkt.map_npc_count;
            existing.state = updated_state;
            existing.lead_species = pkt.lead_species;
            existing.lead_form = pkt.lead_form;
            existing.lead_is_shiny = pkt.lead_is_shiny;
            existing.lead_gender = pkt.lead_gender;
            existing.badge_mask = pkt.badge_mask_local;
        } else {
            let mut initial_state = pkt.local_player;
            initial_state.is_active = 1;
            if initial_state.prev_x == 0 && initial_state.prev_y == 0 {
                initial_state.prev_x = initial_state.x;
                initial_state.prev_y = initial_state.y.saturating_sub(1);
            }

            info!(
                "[JOIN] Assigned Slot P{slot_idx} to client {addr} | Initial Map: {} Pos: ({}, {}) | Lead Species: #{}",
                initial_state.map_id, initial_state.x, initial_state.y, pkt.lead_species
            );

            self.slots[slot_idx] = Some(ConnectedPlayer {
                slot_id: slot_idx as u8,
                addr,
                last_seen: now,
                last_sequence: pkt.sequence,
                heartbeat_rom: pkt.heartbeat_rom,
                map_npc_count: pkt.map_npc_count,
                state: initial_state,
                lead_species: pkt.lead_species,
                lead_form: pkt.lead_form,
                lead_is_shiny: pkt.lead_is_shiny,
                lead_gender: pkt.lead_gender,
                badge_mask: pkt.badge_mask_local,
                assigned_monolocke: 18, // 18 = None (Standard Co-op mode)
            });
        }

        Some(slot_idx as u8)
    }

    fn resolve_or_assign_slot(&self, addr: SocketAddr, requested_id: u8) -> Option<usize> {
        // 1. Check if this exact SocketAddr already owns a slot
        for (idx, slot) in self.slots.iter().enumerate() {
            if let Some(player) = slot {
                if player.addr == addr {
                    return Some(idx);
                }
            }
        }

        // 2. If client requested a specific slot (0..5) and it's free, grant it
        if (requested_id as usize) < MAX_PLAYERS && requested_id != AUTO_PLAYER_ID {
            let idx = requested_id as usize;
            if self.slots[idx].is_none() {
                return Some(idx);
            }
        }

        // 3. Auto-assign lowest available slot (ideal when running multiple emulators on 127.0.0.1)
        for (idx, slot) in self.slots.iter().enumerate() {
            if slot.is_none() {
                return Some(idx);
            }
        }

        warn!("Lobby full ({MAX_PLAYERS}/{MAX_PLAYERS}) — rejected packet from {addr}");
        None
    }

    /// Prunes clients that haven't sent a packet within `client_timeout`.
    pub fn prune_stale_clients(&mut self) {
        let now = Instant::now();
        for (idx, slot) in self.slots.iter_mut().enumerate() {
            if let Some(player) = slot {
                if now.duration_since(player.last_seen) > self.client_timeout {
                    info!(
                        "[TIMEOUT] Slot P{idx} ({}) timed out after {:.1}s",
                        player.addr,
                        self.client_timeout.as_secs_f32()
                    );
                    *slot = None;
                }
            }
        }
    }

    /// Builds customized 30 Hz broadcast packets for every active client.
    pub fn build_broadcast_batch(&mut self) -> Vec<(SocketAddr, [u8; crate::protocol::SERVER_PACKET_SIZE])> {
        self.server_tick = self.server_tick.wrapping_add(1);

        let active_mask = self.compute_active_bitmask();
        let (synced_badges, sync_ready) = self.compute_progression_gates();

        let mut batch = Vec::with_capacity(MAX_PLAYERS);

        for recipient_idx in 0..MAX_PLAYERS {
            let Some(recipient) = &self.slots[recipient_idx] else {
                continue;
            };

            let mut remote_players = [PlayerState::default(); REMOTE_SLOTS];
            let mut remote_followers = [FollowerState::default(); REMOTE_SLOTS];

            let mut remote_cursor = 0;
            for other_idx in 0..MAX_PLAYERS {
                if other_idx == recipient_idx {
                    continue;
                }

                if let Some(other) = &self.slots[other_idx] {
                    let same_map = other.state.map_id == recipient.state.map_id && other.state.map_id != 0;

                    let mut r_player = other.state;
                    // Room filtering: only mark ghost active in ROM if on the exact same mapId
                    r_player.is_active = u8::from(same_map);
                    remote_players[remote_cursor] = r_player;

                    // Follower culling: hide remote followers if map differs, species is 0, or NPC count > 26
                    let show_follower = same_map
                        && other.lead_species > 0
                        && recipient.map_npc_count <= MAX_NPCS_BEFORE_FOLLOWER_CULL;

                    remote_followers[remote_cursor] = FollowerState {
                        species: other.lead_species,
                        form: other.lead_form,
                        is_shiny: other.lead_is_shiny,
                        gender: other.lead_gender,
                        x: other.state.prev_x,
                        y: other.state.prev_y,
                        direction: other.state.direction,
                        is_visible: u8::from(show_follower),
                        pad: [0u8; 12],
                    };
                }

                remote_cursor += 1;
            }

            let pkt = ServerToClientPacket {
                magic: COOP_MAGIC,
                version: PROTOCOL_VERSION,
                assigned_player_id: recipient.slot_id,
                active_players_mask: active_mask,
                server_tick: self.server_tick,
                reserved_header: 0,
                remote_players,
                remote_followers,
                badge_mask_local_echo: recipient.badge_mask,
                badge_mask_synced: synced_badges,
                flag_coop_sync_ready: u8::from(sync_ready),
                current_checkpoint: synced_badges.count_ones() as u8,
                assigned_monolocke: recipient.assigned_monolocke,
                nuzlocke_failed: 0,
                gates_pad: [0u8; 16],
            };

            batch.push((recipient.addr, pkt.encode_le()));
        }

        batch
    }

    pub fn print_periodic_summary(&self) {
        let connected: Vec<&ConnectedPlayer> = self.slots.iter().filter_map(|s| s.as_ref()).collect();
        if connected.is_empty() {
            return;
        }
        let (synced_badges, ready) = self.compute_progression_gates();
        info!(
            "--- [LOBBY SUMMARY: {}/{} Connected | Synced Badges: 0b{:08b} | Gate Ready: {}] ---",
            connected.len(),
            MAX_PLAYERS,
            synced_badges,
            ready
        );
        for p in connected {
            info!(
                "    Slot P{} ({}) -> Map {:3} | XY: ({:3}, {:3}) | Lead: #{:4} | ROM_HB: {}",
                p.slot_id, p.addr, p.state.map_id, p.state.x, p.state.y, p.lead_species, p.heartbeat_rom
            );
        }
    }

    fn compute_active_bitmask(&self) -> u8 {
        let mut mask = 0u8;
        for (idx, slot) in self.slots.iter().enumerate() {
            if slot.is_some() {
                mask |= 1 << idx;
            }
        }
        mask
    }

    fn compute_progression_gates(&self) -> (u16, bool) {
        let active: Vec<&ConnectedPlayer> = self.slots.iter().filter_map(|s| s.as_ref()).collect();
        if active.is_empty() {
            return (0, false);
        }

        // Common badges earned by ALL connected players
        let mut common_badges = 0xFFFFu16;
        let first_map = active[0].state.map_id;
        let mut all_same_map = true;
        let mut all_equal_badges = true;
        let first_badges = active[0].badge_mask;

        for p in &active {
            common_badges &= p.badge_mask;
            if p.state.map_id != first_map {
                all_same_map = false;
            }
            if p.badge_mask != first_badges {
                all_equal_badges = false;
            }
        }

        // Gate opens when at least `min_players_for_sync` are online, have equal badges, and are at the same map
        let sync_ready =
            active.len() >= self.min_players_for_sync && all_equal_badges && all_same_map && first_map != 0;

        (common_badges, sync_ready)
    }

    fn dir_name(dir: u8) -> &'static str {
        match dir {
            0 => "Down",
            1 => "Up",
            2 => "Left",
            3 => "Right",
            _ => "Unknown",
        }
    }
}
