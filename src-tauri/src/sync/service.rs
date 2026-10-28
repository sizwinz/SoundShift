use super::{
    model::*,
    planner,
    runner::{self, Database, Providers},
    store,
};
use crate::engine::{
    matcher::{match_track, MatchClassification},
    rate_limiter::SearchRateLimiter,
};

pub async fn preview(
    db: &Database,
    id: &str,
    providers: &Providers,
    resolution: Option<SyncResolution>,
) -> Result<SyncPlan, String> {
    let pair = {
        let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
        crate::storage::recovery::ensure_mutations_allowed(&conn)?;
        let pair = store::get_pair(&conn, id)?;
        store::validate_generations(&conn, &pair)?;
        pair
    };
    if pair.state == "unlinked" {
        return Err("This playlist pair has been unlinked".into());
    }
    let mut reads = vec![];
    for side in Side::ALL {
        let provider = &providers[side.index()];
        provider.check_session().await.map_err(|e| e.to_string())?;
        provider
            .check_playlist_editable(&pair.endpoint(side).playlist_id)
            .await
            .map_err(|e| e.to_string())?;
        let read = provider
            .read_playlist(&pair.endpoint(side).playlist_id)
            .await
            .map_err(|e| e.to_string())?;
        runner::validate_read(&read)?;
        reads.push(read);
    }
    let mut plan = if let Some(resolution) = resolution {
        let old = {
            let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
            store::load_preview(&conn, &resolution.plan_id)?
        };
        if old.pair_id != id
            || !runner::same_state(&old.spotify_before, &reads[0])
            || !runner::same_state(&old.ytmusic_before, &reads[1])
        {
            return Err("Sync preview is stale; review fresh playlist states".into());
        }
        let mut updated = old;
        updated.plan_id = new_id("plan");
        let order = if let Some(side) = resolution.strategy {
            let mut pool = updated.desired.clone();
            if let Some(baseline) = &pair.baseline {
                for o in &baseline.occurrences {
                    if !pool.iter().any(|p| p.logical_id == o.logical_id) {
                        pool.push(o.clone());
                    }
                }
            }
            let mut used = std::collections::HashSet::new();
            let mut version = vec![];
            for observed in &reads[side.index()].occurrences {
                let entry = pool
                    .iter()
                    .find(|o| {
                        !used.contains(&o.logical_id)
                            && o.track(side).is_some_and(|t| t.id == observed.track.id)
                            && o.remote(side)
                                .is_none_or(|id| Some(id) == observed.remote_id.as_deref())
                    })
                    .cloned()
                    .ok_or("Chosen side contains an unattributed occurrence; review manually")?;
                used.insert(entry.logical_id.clone());
                version.push(entry);
            }
            let order = version.iter().map(|o| o.logical_id.clone()).collect();
            updated.desired = version;
            order
        } else {
            resolution.order
        };
        let mut seen = std::collections::HashSet::new();
        let mut desired = vec![];
        for logical in order {
            if !seen.insert(logical.clone()) {
                return Err("Merged order contains a repeated occurrence identity".into());
            }
            let mut entry = updated
                .desired
                .iter()
                .find(|o| o.logical_id == logical)
                .cloned()
                .ok_or("Merged order contains an unknown occurrence")?;
            for choice in resolution
                .choices
                .iter()
                .filter(|c| c.logical_id == logical)
            {
                if choice.track.id.is_empty() || !choice.track.is_playable {
                    return Err("Choose a playable destination track".into());
                }
                entry.set_track(choice.side, choice.track.clone());
            }
            desired.push(entry);
        }
        updated.desired = desired;
        updated.conflicts.retain(|c| {
            if [
                "ordering_conflict",
                "anchor_conflict",
                "removal_order_conflict",
                "concurrent_addition",
            ]
            .contains(&c.code.as_str())
            {
                return false;
            }
            if c.code == "match_required" {
                return c
                    .logical_id
                    .as_ref()
                    .and_then(|id| updated.desired.iter().find(|o| o.logical_id == *id))
                    .is_some_and(|o| o.spotify.is_none() || o.ytmusic.is_none());
            }
            true
        });
        updated
    } else if let Some(baseline) = &pair.baseline {
        planner::incremental_merge(id, baseline, &reads[0], &reads[1])
    } else {
        planner::initial_merge(id, &reads[0], &reads[1])
    };
    let limiter = SearchRateLimiter::default();
    for entry in &mut plan.desired {
        for target in Side::ALL {
            if entry.track(target).is_some() {
                continue;
            }
            let source = entry
                .track(target.other())
                .ok_or("Source mapping missing")?
                .clone();
            let mut approved = std::collections::HashMap::new();
            for mapping in pair
                .approved_mappings
                .iter()
                .chain(pair.baseline.iter().flat_map(|b| &b.occurrences))
            {
                if mapping
                    .track(target.other())
                    .is_some_and(|t| t.id == source.id)
                {
                    if let Some(track) = mapping.track(target) {
                        approved.insert(track.id.clone(), track.clone());
                    }
                }
            }
            if approved.len() == 1 {
                entry.set_track(target, approved.into_values().next().unwrap());
                continue;
            }
            if approved.len() > 1 {
                let mut conflict=SyncConflict::new("match_required","More than one recording mapping was approved previously; choose the intended recording");
                conflict.logical_id = Some(entry.logical_id.clone());
                conflict.candidates = approved.into_values().collect();
                plan.conflicts.push(conflict);
                continue;
            }
            let matched = match_track(
                &source,
                target.other().service(),
                target.service(),
                providers[target.index()].as_ref(),
                None,
                &limiter,
            )
            .await;
            if matched.status == MatchClassification::Exact {
                if let Some(track) = matched.matched_track {
                    entry.set_track(target, track);
                    continue;
                }
            }
            let mut conflict = SyncConflict::new(
                "match_required",
                "Choose a destination match or exclude this occurrence before syncing",
            );
            conflict.logical_id = Some(entry.logical_id.clone());
            conflict.candidates = matched.candidates.into_iter().map(|c| c.track).collect();
            plan.conflicts.push(conflict);
        }
    }
    let conn = db.lock().map_err(|_| "Storage lock unavailable")?;
    store::save_preview(&conn, &plan)
}
