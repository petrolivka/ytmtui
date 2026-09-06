//! Parser tests against captured InnerTube responses.
//!
//! These exist because R1 - "response shapes change without notice" - is the
//! project's most persistent risk. A shape change should fail here, loudly and
//! specifically, rather than showing up as a silently empty pane.
//!
//! Fixtures are anonymous captures with tracking fields stripped; regenerate
//! them with `cargo run --release --bin dump-fixtures`.

use serde_json::Value;
use ytm_api::parse;
use ytm_core::Row;

fn load(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
    let body = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("missing fixture {path}: {e} - run `cargo run --bin dump-fixtures`")
    });
    serde_json::from_str(&body).unwrap_or_else(|e| panic!("fixture {name} is not JSON: {e}"))
}

fn counts(rows: &[Row]) -> (usize, usize, usize, usize) {
    let mut c = (0, 0, 0, 0);
    for r in rows {
        match r {
            Row::Track(_) => c.0 += 1,
            Row::Album(_) => c.1 += 1,
            Row::Artist(_) => c.2 += 1,
            Row::Playlist(_) => c.3 += 1,
            Row::Header(_) | Row::Category(_) => {}
        }
    }
    c
}

fn categories(rows: &[Row]) -> Vec<&ytm_core::CategoryRef> {
    rows.iter()
        .filter_map(|r| match r {
            Row::Category(c) => Some(c),
            _ => None,
        })
        .collect()
}

#[test]
fn search_songs_yields_playable_tracks_with_metadata() {
    let rows = parse::page_rows(&load("search_songs"));
    let (tracks, ..) = counts(&rows);
    assert!(tracks >= 15, "expected a page of songs, got {tracks}");

    let with_duration = rows
        .iter()
        .filter_map(|r| r.as_track())
        .filter(|t| t.duration.is_some())
        .count();
    let with_artist = rows
        .iter()
        .filter_map(|r| r.as_track())
        .filter(|t| !t.artist.trim().is_empty())
        .count();
    // Not all rows are perfect, but a wholesale parsing regression shows up as
    // these collapsing.
    assert!(
        with_duration * 100 / tracks >= 80,
        "only {with_duration}/{tracks} had durations"
    );
    assert!(
        with_artist * 100 / tracks >= 80,
        "only {with_artist}/{tracks} had artists"
    );
    for t in rows.iter().filter_map(|r| r.as_track()) {
        assert!(t.id.is_valid(), "bad video id {:?}", t.id);
        assert!(!t.title.trim().is_empty());
    }
}

#[test]
fn search_albums_yields_albums_not_tracks() {
    let rows = parse::page_rows(&load("search_albums"));
    let (tracks, albums, ..) = counts(&rows);
    assert!(albums >= 10, "expected albums, got {albums}");
    assert_eq!(tracks, 0, "album search should not yield tracks");
    for r in &rows {
        if let Row::Album(a) = r {
            assert!(!a.id.0.is_empty(), "album with no browse id: {}", a.title);
            // The year must never end up in the artist column.
            assert!(
                !(a.artist.len() == 4 && a.artist.chars().all(|c| c.is_ascii_digit())),
                "year {:?} parsed as the artist of {:?}",
                a.artist,
                a.title
            );
        }
    }
}

#[test]
fn search_artists_and_playlists_are_distinguished() {
    let (_, _, artists, _) = counts(&parse::page_rows(&load("search_artists")));
    assert!(artists >= 1, "expected artists, got {artists}");
    let (_, _, _, playlists) = counts(&parse::page_rows(&load("search_playlists")));
    assert!(playlists >= 10, "expected playlists, got {playlists}");
}

#[test]
fn artist_page_has_albums_and_headers() {
    let rows = parse::page_rows(&load("browse_artist"));
    let (_, albums, ..) = counts(&rows);
    assert!(albums >= 5, "expected an artist's albums, got {albums}");
    assert!(
        rows.iter().any(|r| matches!(r, Row::Header(_))),
        "artist pages are built from titled shelves; none were found"
    );
}

#[test]
fn album_page_is_a_tracklist() {
    let v = load("browse_album");
    let rows = parse::flat_rows(&v);
    let (tracks, ..) = counts(&rows);
    assert!(tracks >= 5, "expected an album tracklist, got {tracks}");
}

#[test]
fn watch_next_yields_a_queue() {
    let tracks = parse::flat_rows_from_queue(&load("watch_next"));
    assert!(
        tracks.len() >= 10,
        "expected a radio queue, got {}",
        tracks.len()
    );
    assert!(tracks.iter().all(|t| t.id.is_valid()));
}

/// The rating of the playing track is in the player overlay, not the queue
/// rows. Reading it from the rows reported Indifferent for everything.
#[test]
fn like_status_lives_in_the_player_overlay() {
    let v = load("watch_next");
    assert!(
        v.get("playerOverlays").is_some(),
        "no playerOverlays in the watch-next response; track_state reads likeStatus from there"
    );
}

#[test]
fn discovery_surfaces_parse() {
    for name in [
        "browse_charts",
        "browse_new_releases",
        "browse_explore",
        "browse_home",
        "browse_moods",
    ] {
        let rows = parse::page_rows(&load(name));
        assert!(!rows.is_empty(), "{name} parsed to nothing");
    }
}

#[test]
fn tracks_carry_thumbnails() {
    let rows = parse::page_rows(&load("search_songs"));
    let with_art = rows
        .iter()
        .filter_map(|r| r.as_track())
        .filter(|t| t.thumbnail.is_some())
        .count();
    assert!(with_art > 0, "no track carried a thumbnail URL");
}

/// The moods and genres are the bulk of Explore, and they are the one surface
/// built from `musicNavigationButtonRenderer` rather than from cards. Before
/// that renderer was handled, this page's 39 tiles parsed to nothing at all
/// and Explore showed only the carousels underneath them.
#[test]
fn explore_yields_its_mood_and_genre_tiles() {
    let rows = parse::page_rows(&load("browse_explore"));
    let cats = categories(&rows);
    assert!(
        cats.len() >= 30,
        "expected the mood and genre tiles, got {}",
        cats.len()
    );

    for want in ["Chill", "Focus", "Workout"] {
        assert!(
            cats.iter().any(|c| c.title == want),
            "{want} missing from Explore"
        );
    }

    // Every mood and genre shares one browse id, so a tile without params is
    // not a destination - it would silently open the wrong page.
    for c in cats
        .iter()
        .filter(|c| c.id.0 == "FEmusic_moods_and_genres_category")
    {
        assert!(
            c.params.as_deref().is_some_and(|p| !p.is_empty()),
            "category {:?} carries no params",
            c.title
        );
    }

    // The three tiles at the top of Explore are ordinary destinations, not
    // categories, and must not be given params they do not have.
    for (title, id) in [
        ("New releases", "FEmusic_new_releases"),
        ("Moods & genres", "FEmusic_moods_and_genres"),
    ] {
        let c = cats
            .iter()
            .find(|c| c.title == title)
            .unwrap_or_else(|| panic!("{title} tile missing"));
        assert_eq!(c.id.0, id);
    }
}

/// The dedicated Moods & genres page, which is where the Explore tile leads.
/// Two grids with their own headers, so the split the official app shows
/// survives into the row list.
#[test]
fn moods_page_splits_moods_from_genres() {
    let rows = parse::page_rows(&load("browse_moods"));
    let cats = categories(&rows);
    assert!(
        cats.len() >= 30,
        "expected every mood and genre, got {}",
        cats.len()
    );
    assert!(
        cats.iter().all(|c| c.params.is_some()),
        "a category came through without its params"
    );

    let headers: Vec<&str> = rows
        .iter()
        .filter_map(|r| match r {
            Row::Header(h) => Some(h.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        headers.iter().any(|h| h.contains("Moods")),
        "no moods header in {headers:?}"
    );
    assert!(
        headers.iter().any(|h| h.contains("Genres")),
        "no genres header in {headers:?}"
    );
}

/// Home's header chips - the mood filters. The response marks none of them
/// selected and offers no "All", so the unfiltered entry is synthesised; if it
/// stopped being first, cycling would never get back to the plain feed.
#[test]
fn home_carries_its_mood_filter_chips() {
    let v = load("browse_home");
    let filters = parse::page_filters(&v);
    assert!(
        filters.len() >= 5,
        "expected Home's mood chips, got {filters:?}"
    );

    assert_eq!(filters[0].label, "All");
    assert!(
        filters[0].params.is_none(),
        "the All chip must be unfiltered"
    );

    for want in ["Workout", "Focus", "Relax"] {
        assert!(
            filters.iter().any(|f| f.label == want),
            "{want} chip missing from {filters:?}"
        );
    }
    // A chip with no params would re-fetch the unfiltered feed under a mood's
    // name, which looks like the filter silently doing nothing.
    for f in filters.iter().skip(1) {
        assert!(
            f.params.as_deref().is_some_and(|p| !p.is_empty()),
            "chip {:?} carries no params",
            f.label
        );
    }

    // The chips must not have come at the cost of the feed itself.
    let rows = parse::page_rows(&v);
    let (tracks, albums, _, playlists) = counts(&rows);
    assert!(
        tracks + albums + playlists >= 10,
        "Home parsed to almost nothing: {tracks} tracks, {albums} albums, {playlists} playlists"
    );
}

/// Surfaces that have no chip cloud must report none rather than picking up
/// something chip-shaped from elsewhere in the response.
#[test]
fn pages_without_a_chip_cloud_have_no_filters() {
    for name in ["browse_moods", "browse_album", "search_songs"] {
        assert!(
            parse::page_filters(&load(name)).is_empty(),
            "{name} produced filters it does not have"
        );
    }
}
