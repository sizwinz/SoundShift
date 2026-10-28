//! Pure occurrence-aware merge planning. This module never dispatches mutations.
use super::model::*;
use crate::models::SourceTrack;
use crate::providers::contracts::PlaylistRead;

pub fn equivalent(a: &SourceTrack, b: &SourceTrack) -> bool {
    a.is_explicit == b.is_explicit
        && a.duration_ms.abs_diff(b.duration_ms) <= 1000
        && !a.title.trim().is_empty()
        && a.title.trim().eq_ignore_ascii_case(b.title.trim())
        && !a.artists.is_empty()
        && a.artists
            .iter()
            .map(|s| s.to_lowercase())
            .collect::<Vec<_>>()
            == b.artists
                .iter()
                .map(|s| s.to_lowercase())
                .collect::<Vec<_>>()
}
pub fn initial_merge(pair_id: &str, spotify: &PlaylistRead, ytmusic: &PlaylistRead) -> SyncPlan {
    let mut used = std::collections::HashSet::new();
    let mut desired = vec![];
    for a in &spotify.occurrences {
        let b = ytmusic
            .occurrences
            .iter()
            .find(|b| !used.contains(&b.index) && equivalent(&a.track, &b.track));
        if let Some(b) = b {
            used.insert(b.index);
        }
        desired.push(LinkedOccurrence {
            logical_id: new_id("occ"),
            spotify: Some(a.track.clone()),
            spotify_remote_id: a.remote_id.clone(),
            ytmusic: b.map(|b| b.track.clone()),
            ytmusic_remote_id: b.and_then(|b| b.remote_id.clone()),
        });
    }
    for b in &ytmusic.occurrences {
        if !used.contains(&b.index) {
            desired.push(LinkedOccurrence {
                logical_id: new_id("occ"),
                spotify: None,
                spotify_remote_id: None,
                ytmusic: Some(b.track.clone()),
                ytmusic_remote_id: b.remote_id.clone(),
            });
        }
    }
    SyncPlan {
        plan_id: new_id("plan"),
        pair_id: pair_id.into(),
        baseline_version: 0,
        pair_stamp: String::new(),
        spotify_before: spotify.clone(),
        ytmusic_before: ytmusic.clone(),
        desired,
        conflicts: vec![],
        initial: true,
    }
}

fn project(baseline: &SyncBaseline, side: Side, current: &PlaylistRead) -> Vec<LinkedOccurrence> {
    let mut used = std::collections::HashSet::new();
    current
        .occurrences
        .iter()
        .map(|o| {
            let existing = baseline.occurrences.iter().find(|b| {
                !used.contains(&b.logical_id)
                    && b.track(side).is_some_and(|t| t.id == o.track.id)
                    && (b.remote(side).is_none() || b.remote(side) == o.remote_id.as_deref())
            });
            if let Some(b) = existing {
                used.insert(b.logical_id.clone());
                b.clone()
            } else {
                let mut entry = LinkedOccurrence {
                    logical_id: new_id("occ"),
                    spotify: None,
                    ytmusic: None,
                    spotify_remote_id: None,
                    ytmusic_remote_id: None,
                };
                entry.set_track(side, o.track.clone());
                entry.set_remote(side, o.remote_id.clone());
                entry
            }
        })
        .collect()
}

pub fn incremental_merge(
    pair_id: &str,
    baseline: &SyncBaseline,
    spotify: &PlaylistRead,
    ytmusic: &PlaylistRead,
) -> SyncPlan {
    let a = project(baseline, Side::Spotify, spotify);
    let b = project(baseline, Side::Ytmusic, ytmusic);
    let baseline_ids: std::collections::HashSet<_> = baseline
        .occurrences
        .iter()
        .map(|o| o.logical_id.as_str())
        .collect();
    let a_ids: std::collections::HashSet<_> = a.iter().map(|o| o.logical_id.as_str()).collect();
    let b_ids: std::collections::HashSet<_> = b.iter().map(|o| o.logical_id.as_str()).collect();
    let mut desired: Vec<_> = baseline
        .occurrences
        .iter()
        .filter(|o| a_ids.contains(o.logical_id.as_str()) && b_ids.contains(o.logical_id.as_str()))
        .cloned()
        .collect();
    let survivors: std::collections::HashSet<_> =
        desired.iter().map(|o| o.logical_id.clone()).collect();
    let a_order: Vec<_> = a
        .iter()
        .filter(|o| survivors.contains(&o.logical_id))
        .map(|o| o.logical_id.clone())
        .collect();
    let old_order: Vec<_> = desired.iter().map(|o| o.logical_id.clone()).collect();
    let b_order: Vec<_> = b
        .iter()
        .filter(|o| survivors.contains(&o.logical_id))
        .map(|o| o.logical_id.clone())
        .collect();
    let mut conflicts = vec![];
    if a.iter()
        .filter(|o| !baseline_ids.contains(o.logical_id.as_str()))
        .any(|left| {
            b.iter()
                .filter(|o| !baseline_ids.contains(o.logical_id.as_str()))
                .any(|right| {
                    left.spotify
                        .as_ref()
                        .zip(right.ytmusic.as_ref())
                        .is_some_and(|(a, b)| equivalent(a, b))
                })
        })
    {
        conflicts.push(SyncConflict::new("concurrent_addition","The same new recording was added independently on both services; review whether to keep one occurrence or both"));
    }
    let reordered = |current: &[LinkedOccurrence], present: &std::collections::HashSet<&str>| {
        let actual: Vec<_> = current
            .iter()
            .filter(|o| baseline_ids.contains(o.logical_id.as_str()))
            .map(|o| o.logical_id.as_str())
            .collect();
        let expected: Vec<_> = baseline
            .occurrences
            .iter()
            .filter(|o| present.contains(o.logical_id.as_str()))
            .map(|o| o.logical_id.as_str())
            .collect();
        actual != expected
    };
    if (baseline
        .occurrences
        .iter()
        .any(|o| !a_ids.contains(o.logical_id.as_str()))
        && reordered(&b, &b_ids))
        || (baseline
            .occurrences
            .iter()
            .any(|o| !b_ids.contains(o.logical_id.as_str()))
            && reordered(&a, &a_ids))
    {
        conflicts.push(SyncConflict::new(
            "removal_order_conflict",
            "A removal overlaps concurrent ordering edits; review both playlists before writing",
        ));
    }
    let mut seen = std::collections::HashSet::new();
    if baseline
        .spotify
        .occurrences
        .iter()
        .any(|o| !seen.insert(&o.track.id))
        && (baseline.spotify.revision != spotify.revision
            || baseline.spotify.ids() != spotify.ids())
    {
        conflicts.push(SyncConflict::new("duplicate_attribution","Spotify duplicate occurrences changed without stable occurrence identities; manual attribution required"));
    }
    if a_order != old_order && b_order != old_order && a_order != b_order {
        conflicts.push(SyncConflict::new(
            "ordering_conflict",
            "Both playlists changed their order differently; choose an order before writing",
        ));
    }
    let order = if a_order != old_order {
        a_order
    } else {
        b_order
    };
    desired.sort_by_key(|o| {
        order
            .iter()
            .position(|id| id == &o.logical_id)
            .unwrap_or(usize::MAX)
    });
    let mut insertions = vec![Vec::new(); desired.len() + 1];
    let positions: std::collections::HashMap<_, _> = desired
        .iter()
        .enumerate()
        .map(|(i, o)| (o.logical_id.clone(), i))
        .collect();
    for current in [&a, &b] {
        for (index, o) in current
            .iter()
            .enumerate()
            .filter(|(_, o)| !baseline_ids.contains(o.logical_id.as_str()))
        {
            let previous = current[..index]
                .iter()
                .rev()
                .find_map(|o| positions.get(&o.logical_id).copied());
            let next = current[index + 1..]
                .iter()
                .find_map(|o| positions.get(&o.logical_id).copied());
            if previous.zip(next).is_some_and(|(p, n)| p >= n) {
                conflicts.push(SyncConflict::new(
                    "anchor_conflict",
                    "Insertion anchors changed incompatibly; review the merged order",
                ));
            }
            insertions[next.unwrap_or(desired.len())].push(o.clone());
        }
    }
    let mut merged = vec![];
    for (i, o) in desired.into_iter().enumerate() {
        merged.append(&mut insertions[i]);
        merged.push(o);
    }
    merged.append(insertions.last_mut().unwrap());
    let desired = merged;
    SyncPlan {
        plan_id: new_id("plan"),
        pair_id: pair_id.into(),
        baseline_version: baseline.version,
        pair_stamp: String::new(),
        spotify_before: spotify.clone(),
        ytmusic_before: ytmusic.clone(),
        desired,
        conflicts,
        initial: false,
    }
}
