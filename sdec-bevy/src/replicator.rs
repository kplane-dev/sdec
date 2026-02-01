use std::time::Instant;

use anyhow::{anyhow, Result};
use bevy_ecs::prelude::World;
use codec::{
    decode_session_packet, encode_delta_from_changes, CodecLimits, SessionEncoder, SessionState,
};
use wire::{decode_packet, Limits as WireLimits};

use crate::apply::apply_changes;
use crate::extract::extract_changes;
use crate::mapping::EntityMap;
use crate::metrics::{EncodeMetrics, MetricsSink};
use crate::schema::BevySchema;

pub struct BevyReplicator {
    schema: BevySchema,
    limits: CodecLimits,
    wire_limits: WireLimits,
    entities: EntityMap,
    session: Option<SessionState>,
    metrics: Option<Box<dyn MetricsSink>>,
}

impl BevyReplicator {
    #[must_use]
    pub fn new(schema: BevySchema) -> Self {
        Self {
            schema,
            limits: CodecLimits::default(),
            wire_limits: WireLimits::default(),
            entities: EntityMap::new(),
            session: None,
            metrics: None,
        }
    }

    pub fn with_limits(mut self, limits: CodecLimits, wire_limits: WireLimits) -> Self {
        self.limits = limits;
        self.wire_limits = wire_limits;
        self
    }

    pub fn set_metrics_sink(&mut self, sink: Box<dyn MetricsSink>) {
        self.metrics = Some(sink);
    }

    pub fn update_session(&mut self, session: SessionState) {
        self.session = Some(session);
    }

    pub fn encode_frame(
        &mut self,
        world: &mut World,
        tick: codec::SnapshotTick,
        baseline_tick: codec::SnapshotTick,
        out: &mut [u8],
    ) -> Result<usize> {
        let changes = extract_changes(&self.schema, world, &mut self.entities);
        let mut encoder = SessionEncoder::new(self.schema.schema(), &self.limits);
        let start = Instant::now();
        let bytes = encode_delta_from_changes(
            &mut encoder,
            tick,
            baseline_tick,
            &changes.creates,
            &changes.destroys,
            &changes.updates,
            out,
        )?;
        if let Some(metrics) = self.metrics.as_mut() {
            metrics.record_encode(EncodeMetrics {
                bytes,
                encode_time: start.elapsed(),
            });
        }
        Ok(bytes)
    }

    pub fn apply_frame(&mut self, world: &mut World, bytes: &[u8]) -> Result<()> {
        let packet = decode_packet(bytes, &self.wire_limits)?;
        let decoded = codec::decode_delta_packet(self.schema.schema(), &packet, &self.limits)?;
        apply_changes(
            &self.schema,
            world,
            &mut self.entities,
            &decoded.creates,
            &decoded.destroys,
            &decoded.updates,
        )?;
        Ok(())
    }

    pub fn apply_compact_frame(&mut self, world: &mut World, bytes: &[u8]) -> Result<()> {
        let Some(session) = self.session.as_mut() else {
            return Err(anyhow!("session state missing; call update_session first"));
        };
        let packet =
            decode_session_packet(self.schema.schema(), session, bytes, &self.wire_limits)?;
        let decoded = codec::decode_delta_packet(self.schema.schema(), &packet, &self.limits)?;
        apply_changes(
            &self.schema,
            world,
            &mut self.entities,
            &decoded.creates,
            &decoded.destroys,
            &decoded.updates,
        )?;
        Ok(())
    }
}
