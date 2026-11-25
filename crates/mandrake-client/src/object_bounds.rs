use std::collections::BTreeMap;

use mandrake_common::FourCC;
use mandrake_sim::Simulation;
use mandrake_sim::engine_rng::{TagSource, equalize_collection_for_palette};
use mandrake_sim::mons_postprocess::{MonsPostprocess, PostprocessedTags};
use mandrake_tag::{MeshTag, SequenceWorldBounds};
use mandrake_view::selection::ObjectBounds;
use tracing::{debug, warn};

use crate::object_history::interpolate_i16;

pub fn mons_postprocess_of(sim: &Simulation) -> MonsPostprocess {
    sim.engine.mons_postprocess
}

pub struct SequenceBoundsTable<'a> {
    mesh: &'a MeshTag,
    tags: PostprocessedTags<'a>,
    collections: BTreeMap<FourCC, Vec<SequenceWorldBounds>>,
}

impl<'a> SequenceBoundsTable<'a> {
    pub fn new(mesh: &'a MeshTag, tags: &'a dyn TagSource, postprocess: MonsPostprocess) -> Self {
        Self {
            mesh,
            tags: PostprocessedTags { tags, postprocess },
            collections: BTreeMap::new(),
        }
    }

    pub fn world_bounds(
        &mut self,
        collection: FourCC,
        sequence: usize,
        fallback: SequenceWorldBounds,
    ) -> SequenceWorldBounds {
        let (mesh, tags) = (self.mesh, &self.tags);
        self.collections
            .entry(collection)
            .or_insert_with(|| equalized_collection(mesh, tags, collection))
            .get(sequence)
            .copied()
            .unwrap_or(fallback)
    }
}

fn equalized_collection(
    mesh: &MeshTag,
    tags: &dyn TagSource,
    collection: FourCC,
) -> Vec<SequenceWorldBounds> {
    let Some(mut headers) = tags
        .tag(*b".256", collection)
        .and_then(|d| mandrake_tag::sequence_headers(&d).ok())
    else {
        return Vec::new();
    };
    match equalize_collection_for_palette(mesh, tags, collection, &mut headers) {
        Ok(None) => {}
        Ok(Some(order)) => debug!("Collection {collection}: {}", order.reason),
        Err(e) => warn!("Collection {collection} not equalized: {}", e.reason),
    }
    headers.iter().map(|h| h.world_bounds).collect()
}

pub fn interpolated_bounds(
    history: ObjectBounds,
    live: ObjectBounds,
    fraction: i32,
) -> ObjectBounds {
    if fraction > 0xffff {
        return live;
    }
    ObjectBounds {
        half_width: interpolate_i16(history.half_width, live.half_width, fraction),
        bottom: interpolate_i16(history.bottom, live.bottom, fraction),
        top: interpolate_i16(history.top, live.top, fraction),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(half_width: i16, bottom: i16, top: i16) -> ObjectBounds {
        ObjectBounds {
            half_width,
            bottom,
            top,
        }
    }

    #[test]
    fn bounds_go_from_the_history_to_the_live_object() {
        let (h, l) = (bounds(120, -70, 580), bounds(100, -30, 500));
        assert_eq!(interpolated_bounds(h, l, 0), h);
        assert_eq!(interpolated_bounds(h, l, 0x8000), bounds(110, -50, 540));
        assert_eq!(interpolated_bounds(h, l, 0x10000), l);
        assert_eq!(interpolated_bounds(h, l, 1).half_width, 119);
    }
}
