use async_trait::async_trait;
use serde::Deserialize;
use soundshift_lib::engine::matcher::{match_track, MatchClassification};
use soundshift_lib::engine::rate_limiter::SearchRateLimiter;
use soundshift_lib::models::{Playlist, SourceTrack};
use soundshift_lib::providers::traits::MusicProvider;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct BenchmarkFixture {
    id: String,
    category: String,
    source: SourceTrack,
    candidates: Vec<SourceTrack>,
    expected_status: String,
    expected_best_id: Option<String>,
}

struct MockBenchmarkProvider {
    candidates: Vec<SourceTrack>,
}

#[async_trait]
impl MusicProvider for MockBenchmarkProvider {
    async fn list_playlists(&self) -> Result<Vec<Playlist>, String> {
        Ok(vec![])
    }

    async fn get_playlist_tracks(
        &self,
        _playlist_id: &str,
        _tx: Option<tokio::sync::mpsc::Sender<Vec<SourceTrack>>>,
    ) -> Result<Vec<SourceTrack>, String> {
        Ok(vec![])
    }

    async fn search_track(&self, _query: &str) -> Result<Vec<SourceTrack>, String> {
        Ok(self.candidates.clone())
    }

    async fn create_playlist(
        &self,
        _title: &str,
        _description: Option<&str>,
    ) -> Result<String, String> {
        Ok("mock_playlist_id".to_string())
    }

    async fn add_tracks_to_playlist(
        &self,
        _playlist_id: &str,
        _track_ids: &[String],
    ) -> Result<(), String> {
        Ok(())
    }

    async fn remove_tracks_from_playlist(
        &self,
        _playlist_id: &str,
        _track_ids: &[String],
    ) -> Result<(), String> {
        Ok(())
    }

    async fn delete_playlist(&self, _playlist_id: &str) -> Result<(), String> {
        Ok(())
    }
}

#[tokio::test]
async fn test_headless_matching_benchmark() {
    let fixture_path = Path::new("tests/fixtures/benchmark_tracks.json");
    let content = fs::read_to_string(fixture_path)
        .or_else(|_| fs::read_to_string("src-tauri/tests/fixtures/benchmark_tracks.json"))
        .expect("Failed to load benchmark fixture file");

    let fixtures: Vec<BenchmarkFixture> =
        serde_json::from_str(&content).expect("Failed to parse benchmark json");

    assert!(
        fixtures.len() >= 100,
        "Benchmark fixture suite must contain at least 100 tracks, found {}",
        fixtures.len()
    );

    let rate_limiter = SearchRateLimiter::new(10, 0); // Fast unthrottled execution for offline benchmark

    let mut total_tracks = 0;
    let mut total_studio = 0;
    let mut correct_studio = 0;

    let mut total_remaster = 0;
    let mut correct_remaster = 0;

    let mut total_feat = 0;
    let mut correct_feat = 0;

    let mut total_diacritics = 0;
    let mut correct_diacritics = 0;

    let mut total_inverted = 0;
    let mut correct_inverted = 0;

    let mut total_negative = 0;
    let mut rejected_negative = 0;

    let mut total_ambiguous = 0;
    let mut correct_ambiguous = 0;

    for fixture in &fixtures {
        total_tracks += 1;
        let mock_provider = MockBenchmarkProvider {
            candidates: fixture.candidates.clone(),
        };

        let result = match_track(
            &fixture.source,
            "spotify",
            "ytmusic",
            &mock_provider,
            None,
            &rate_limiter,
        )
        .await;

        match fixture.category.as_str() {
            "studio" => {
                total_studio += 1;
                if result.status == MatchClassification::Exact {
                    if let Some(ref matched) = result.matched_track {
                        if Some(&matched.id) == fixture.expected_best_id.as_ref() {
                            correct_studio += 1;
                        }
                    }
                }
            }
            "remaster" => {
                total_remaster += 1;
                if result.status == MatchClassification::Exact {
                    if let Some(ref matched) = result.matched_track {
                        if Some(&matched.id) == fixture.expected_best_id.as_ref() {
                            correct_remaster += 1;
                        }
                    }
                }
            }
            "featured" => {
                total_feat += 1;
                if result.status == MatchClassification::Exact {
                    if let Some(ref matched) = result.matched_track {
                        if Some(&matched.id) == fixture.expected_best_id.as_ref() {
                            correct_feat += 1;
                        }
                    }
                }
            }
            "diacritics" => {
                total_diacritics += 1;
                if result.status == MatchClassification::Exact {
                    if let Some(ref matched) = result.matched_track {
                        if Some(&matched.id) == fixture.expected_best_id.as_ref() {
                            correct_diacritics += 1;
                        }
                    }
                }
            }
            "inverted" => {
                total_inverted += 1;
                if result.status == MatchClassification::Exact {
                    if let Some(ref matched) = result.matched_track {
                        if Some(&matched.id) == fixture.expected_best_id.as_ref() {
                            correct_inverted += 1;
                        }
                    }
                }
            }
            "negative_duration" => {
                total_negative += 1;
                // MTCH-06: Strict rejection for duration delta > 15s
                if result.status == MatchClassification::NotFound {
                    rejected_negative += 1;
                }
            }
            "ambiguous" => {
                total_ambiguous += 1;
                // MTCH-05: Ambiguous flag for delta between 4s and 15s
                if result.status == MatchClassification::Ambiguous {
                    correct_ambiguous += 1;
                }
            }
            _ => {}
        }
    }

    let studio_accuracy = (correct_studio as f64) / (total_studio as f64);
    let remaster_accuracy = (correct_remaster as f64) / (total_remaster as f64);
    let feat_accuracy = (correct_feat as f64) / (total_feat as f64);
    let diacritic_accuracy = (correct_diacritics as f64) / (total_diacritics as f64);
    let inverted_accuracy = (correct_inverted as f64) / (total_inverted as f64);
    let negative_rejection_rate = (rejected_negative as f64) / (total_negative as f64);
    let ambiguous_accuracy = (correct_ambiguous as f64) / (total_ambiguous as f64);

    println!("\n==================================================================");
    println!("             SOUNDSHIFT MATCHING BENCHMARK REPORT                 ");
    println!("==================================================================");
    println!("{:<25} | {:<8} | {:<8} | {:<10}", "Category", "Total", "Passed", "Accuracy");
    println!("------------------------------------------------------------------");
    println!("{:<25} | {:<8} | {:<8} | {:.2}%", "Studio Recordings", total_studio, correct_studio, studio_accuracy * 100.0);
    println!("{:<25} | {:<8} | {:<8} | {:.2}%", "Remaster / Deluxe Editions", total_remaster, correct_remaster, remaster_accuracy * 100.0);
    println!("{:<25} | {:<8} | {:<8} | {:.2}%", "Featured Artists In Title", total_feat, correct_feat, feat_accuracy * 100.0);
    println!("{:<25} | {:<8} | {:<8} | {:.2}%", "Diacritics & International", total_diacritics, correct_diacritics, diacritic_accuracy * 100.0);
    println!("{:<25} | {:<8} | {:<8} | {:.2}%", "Inverted / Transposed Words", total_inverted, correct_inverted, inverted_accuracy * 100.0);
    println!("{:<25} | {:<8} | {:<8} | {:.2}%", "Delta > 15s Rejections", total_negative, rejected_negative, negative_rejection_rate * 100.0);
    println!("{:<25} | {:<8} | {:<8} | {:.2}%", "4s < Delta <= 15s Ambiguous", total_ambiguous, correct_ambiguous, ambiguous_accuracy * 100.0);
    println!("==================================================================");
    println!("Total Fixtures Evaluated: {}", total_tracks);
    println!("==================================================================\n");

    // Invariant assertions:
    // 1. Studio track accuracy >= 98%
    assert!(
        studio_accuracy >= 0.98,
        "Studio accuracy fell below 98%: {:.2}%",
        studio_accuracy * 100.0
    );

    // 2. Remaster noise token stripping accuracy >= 95%
    assert!(
        remaster_accuracy >= 0.95,
        "Remaster accuracy fell below 95%: {:.2}%",
        remaster_accuracy * 100.0
    );

    // 3. Featured artist extraction accuracy >= 95%
    assert!(
        feat_accuracy >= 0.95,
        "Featured artist accuracy fell below 95%: {:.2}%",
        feat_accuracy * 100.0
    );

    // 4. Diacritic folding accuracy >= 95%
    assert!(
        diacritic_accuracy >= 0.95,
        "Diacritic accuracy fell below 95%: {:.2}%",
        diacritic_accuracy * 100.0
    );

    // 5. Inverted word ordering accuracy >= 95%
    assert!(
        inverted_accuracy >= 0.95,
        "Inverted word accuracy fell below 95%: {:.2}%",
        inverted_accuracy * 100.0
    );

    // 6. MTCH-06: 100% of candidates with duration delta > 15s must be rejected
    assert_eq!(
        rejected_negative, total_negative,
        "Expected 100% rejection of duration delta > 15s, got {} of {}",
        rejected_negative, total_negative
    );

    // 7. MTCH-05: 100% of candidates with 4s < delta <= 15s must be classified Ambiguous
    assert_eq!(
        correct_ambiguous, total_ambiguous,
        "Expected all 4s < delta <= 15s to be flagged Ambiguous, got {} of {}",
        correct_ambiguous, total_ambiguous
    );
}
