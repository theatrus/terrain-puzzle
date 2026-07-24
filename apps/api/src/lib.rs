use std::{
    collections::HashMap,
    env,
    panic::{AssertUnwindSafe, catch_unwind},
    path::PathBuf,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path as AxumPath, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::Response,
    routing::get,
};
use chrono::{DateTime, Utc};
use reqwest::Client;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use terrain_core::{
    Artifact, GenerationSpec, artifact_path, generate_project_with_fields_cancellable,
};
use tokio::{net::TcpListener, sync::Mutex as AsyncMutex, time::sleep};
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use tracing::{error, info};
use uuid::Uuid;

mod cache;
mod elevation;
mod surface;

#[derive(Clone)]
struct AppState {
    db: Arc<StdMutex<Connection>>,
    jobs_dir: Arc<PathBuf>,
    map_cache_dir: Arc<PathBuf>,
    geocoder: Client,
    geocoder_base_url: Arc<String>,
    last_geocode_request: Arc<AsyncMutex<Instant>>,
    active_jobs: Arc<StdMutex<HashMap<String, Arc<AtomicBool>>>>,
}

#[derive(Debug, Serialize)]
struct Health {
    status: &'static str,
    storage: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Job {
    id: String,
    status: String,
    progress: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    spec: GenerationSpec,
    artifacts: Vec<Artifact>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct ApiError {
    error: String,
}

#[derive(Debug, Deserialize)]
struct PlaceSearch {
    q: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlaceResult {
    display_name: String,
    latitude: f64,
    longitude: f64,
    category: String,
    kind: String,
}

#[derive(Debug, Deserialize)]
struct NominatimPlace {
    display_name: String,
    lat: String,
    lon: String,
    category: String,
    #[serde(rename = "type")]
    kind: String,
}

pub async fn run() -> Result<()> {
    let data_dir = PathBuf::from(env::var("TERRAIN_DATA_DIR").unwrap_or_else(|_| "data".into()));
    let address = env::var("TERRAIN_BIND").unwrap_or_else(|_| "127.0.0.1:8787".into());
    run_with(data_dir, address).await
}

pub async fn run_with(data_dir: PathBuf, address: String) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "terrain_api=info,tower_http=info".into()),
        )
        .try_init()
        .ok();

    let jobs_dir = data_dir.join("jobs");
    let map_cache_dir = cache::root()?;
    std::fs::create_dir_all(&jobs_dir)
        .with_context(|| format!("create jobs directory {}", jobs_dir.display()))?;
    std::fs::create_dir_all(&map_cache_dir)
        .with_context(|| format!("create map cache directory {}", map_cache_dir.display()))?;
    let connection = Connection::open(data_dir.join("toposaic.sqlite3"))?;
    migrate(&connection)?;
    let geocoder = Client::builder()
        .user_agent("toposaic/0.1 (+https://github.com/theatrus/terrain-puzzle)")
        .timeout(Duration::from_secs(12))
        .build()?;

    let state = AppState {
        db: Arc::new(StdMutex::new(connection)),
        jobs_dir: Arc::new(jobs_dir),
        map_cache_dir: Arc::new(map_cache_dir.clone()),
        geocoder,
        geocoder_base_url: Arc::new(
            env::var("NOMINATIM_BASE_URL")
                .unwrap_or_else(|_| "https://nominatim.openstreetmap.org".into()),
        ),
        last_geocode_request: Arc::new(AsyncMutex::new(Instant::now() - Duration::from_secs(1))),
        active_jobs: Arc::new(StdMutex::new(HashMap::new())),
    };

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/places", get(search_places))
        .route("/api/preview", axum::routing::post(create_preview))
        .route("/api/jobs", get(list_jobs).post(create_job))
        .route("/api/jobs/{id}", get(get_job).delete(cancel_job))
        .route("/api/jobs/{id}/downloads/{name}", get(download))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = TcpListener::bind(&address).await?;
    info!(
        %address,
        data_dir = %data_dir.display(),
        map_cache_dir = %map_cache_dir.display(),
        "terrain api ready"
    );
    axum::serve(listener, app).await?;
    Ok(())
}

fn migrate(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;
        CREATE TABLE IF NOT EXISTS jobs (
            id TEXT PRIMARY KEY,
            status TEXT NOT NULL,
            progress INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            spec_json TEXT NOT NULL,
            artifacts_json TEXT NOT NULL DEFAULT '[]',
            error TEXT
        );
        CREATE INDEX IF NOT EXISTS jobs_created_at_idx ON jobs(created_at DESC);
        CREATE TABLE IF NOT EXISTS place_search_cache (
            query TEXT PRIMARY KEY,
            response_json TEXT NOT NULL,
            fetched_at TEXT NOT NULL
        );
        UPDATE jobs
        SET status = 'failed',
            progress = 100,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
            error = 'Generation was interrupted by a service restart.'
        WHERE status IN ('queued', 'running');
        "#,
    )?;
    Ok(())
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        storage: "sqlite",
    })
}

async fn search_places(
    State(state): State<AppState>,
    Query(search): Query<PlaceSearch>,
) -> Result<Json<Vec<PlaceResult>>, (StatusCode, Json<ApiError>)> {
    let query = search.q.trim();
    if !(2..=120).contains(&query.len()) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "place search must be between 2 and 120 characters",
        ));
    }
    let normalized_query = query.to_lowercase();
    if let Some(cached) = find_cached_places(&state, &normalized_query).map_err(internal_error)? {
        return Ok(Json(cached));
    }

    let results = fetch_places(&state, query, &normalized_query)
        .await
        .map_err(internal_error)?;
    Ok(Json(results))
}

fn find_cached_places(state: &AppState, query: &str) -> Result<Option<Vec<PlaceResult>>> {
    let connection = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("database lock failed"))?;
    let mut statement =
        connection.prepare("SELECT response_json FROM place_search_cache WHERE query = ?1")?;
    let mut rows = statement.query([query])?;
    rows.next()?
        .map(|row| {
            let value: String = row.get(0)?;
            serde_json::from_str(&value).map_err(sql_conversion_error)
        })
        .transpose()
        .map_err(Into::into)
}

async fn fetch_places(
    state: &AppState,
    query: &str,
    normalized_query: &str,
) -> Result<Vec<PlaceResult>> {
    {
        let mut previous = state.last_geocode_request.lock().await;
        let wait = Duration::from_secs(1).saturating_sub(previous.elapsed());
        if !wait.is_zero() {
            sleep(wait).await;
        }
        *previous = Instant::now();
    }

    let url = format!("{}/search", state.geocoder_base_url.trim_end_matches('/'));
    let response = state
        .geocoder
        .get(url)
        .query(&[
            ("q", query),
            ("format", "jsonv2"),
            ("limit", "5"),
            ("addressdetails", "0"),
        ])
        .send()
        .await
        .context("search OpenStreetMap places")?
        .error_for_status()
        .context("OpenStreetMap place search failed")?;
    let results = response
        .json::<Vec<NominatimPlace>>()
        .await?
        .into_iter()
        .map(PlaceResult::try_from)
        .collect::<Result<Vec<_>>>()?;

    let connection = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("database lock failed"))?;
    connection.execute(
        "INSERT INTO place_search_cache (query, response_json, fetched_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(query) DO UPDATE SET
             response_json = excluded.response_json,
             fetched_at = excluded.fetched_at",
        params![
            normalized_query,
            serde_json::to_string(&results)?,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(results)
}

impl TryFrom<NominatimPlace> for PlaceResult {
    type Error = anyhow::Error;

    fn try_from(place: NominatimPlace) -> Result<Self> {
        Ok(Self {
            display_name: place.display_name,
            latitude: place.lat.parse().context("invalid place latitude")?,
            longitude: place.lon.parse().context("invalid place longitude")?,
            category: place.category,
            kind: place.kind,
        })
    }
}

async fn create_job(
    State(state): State<AppState>,
    Json(spec): Json<GenerationSpec>,
) -> Result<(StatusCode, Json<Job>), (StatusCode, Json<ApiError>)> {
    spec.validate()
        .map_err(|error| api_error(StatusCode::BAD_REQUEST, error))?;

    let id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let job = Job {
        id: id.clone(),
        status: "queued".into(),
        progress: 0,
        created_at: now,
        updated_at: now,
        spec: spec.clone(),
        artifacts: Vec::new(),
        error: None,
    };
    insert_job(&state, &job).map_err(internal_error)?;

    let cancellation = Arc::new(AtomicBool::new(false));
    state
        .active_jobs
        .lock()
        .map_err(|_| internal_error("active job lock failed"))?
        .insert(id.clone(), cancellation.clone());
    let worker_state = state.clone();
    tokio::task::spawn_blocking(move || {
        let result = catch_unwind(AssertUnwindSafe(|| {
            run_job(&worker_state, &id, &spec, &cancellation)
        }));
        if cancellation.load(Ordering::Acquire) {
            let output_dir = worker_state.jobs_dir.join(&id);
            if let Err(cleanup_error) = std::fs::remove_dir_all(&output_dir)
                && cleanup_error.kind() != std::io::ErrorKind::NotFound
            {
                error!(job_id = %id, error = %cleanup_error, "cancel cleanup failed");
            }
        } else {
            let failure = match result {
                Ok(Ok(())) => None,
                Ok(Err(error)) => Some(error.to_string()),
                Err(payload) => Some(panic_message(payload)),
            };
            if let Some(failure) = failure {
                error!(job_id = %id, error = %failure, "generation failed");
                let _ = update_job(&worker_state, &id, "failed", 100, &[], Some(&failure));
            }
        }
        if let Ok(mut active_jobs) = worker_state.active_jobs.lock() {
            active_jobs.remove(&id);
        }
    });

    Ok((StatusCode::ACCEPTED, Json(job)))
}

async fn create_preview(
    State(state): State<AppState>,
    Json(spec): Json<GenerationSpec>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    spec.validate()
        .map_err(|error| api_error(StatusCode::BAD_REQUEST, error))?;
    let cache_dir = state.map_cache_dir.join("elevation");
    let preview = tokio::task::spawn_blocking(move || {
        let height_field = elevation::fetch_preview_height_field(&spec, &cache_dir, 64)?;
        terrain_core::build_height_preview(&spec, &height_field, 64)
    })
    .await
    .map_err(internal_error)?
    .map_err(internal_error)?;
    Ok(Json(preview))
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        format!("mesh generation panicked: {message}")
    } else if let Some(message) = payload.downcast_ref::<String>() {
        format!("mesh generation panicked: {message}")
    } else {
        "mesh generation panicked".into()
    }
}

async fn get_job(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Job>, (StatusCode, Json<ApiError>)> {
    find_job(&state, &id)
        .map_err(internal_error)?
        .map(Json)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "job not found"))
}

async fn cancel_job(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Job>, (StatusCode, Json<ApiError>)> {
    let id =
        canonical_job_id(&id).ok_or_else(|| api_error(StatusCode::NOT_FOUND, "job not found"))?;
    let job = find_job(&state, &id)
        .map_err(internal_error)?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "job not found"))?;
    if !matches!(job.status.as_str(), "queued" | "running") {
        return Err(api_error(StatusCode::CONFLICT, "job is no longer running"));
    }

    if !mark_job_canceled(&state, &id).map_err(internal_error)? {
        return Err(api_error(StatusCode::CONFLICT, "job is no longer running"));
    }
    if let Some(cancellation) = state
        .active_jobs
        .lock()
        .map_err(|_| internal_error("active job lock failed"))?
        .get(&id)
    {
        cancellation.store(true, Ordering::Release);
    }
    find_job(&state, &id)
        .map_err(internal_error)?
        .map(Json)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "job not found"))
}

async fn list_jobs(
    State(state): State<AppState>,
) -> Result<Json<Vec<Job>>, (StatusCode, Json<ApiError>)> {
    let connection = state
        .db
        .lock()
        .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "database lock failed"))?;
    let mut statement = connection
        .prepare(
            "SELECT id, status, progress, created_at, updated_at, spec_json, artifacts_json, error
             FROM jobs ORDER BY created_at DESC LIMIT 20",
        )
        .map_err(internal_error)?;
    let rows = statement
        .query_map([], row_to_job)
        .map_err(internal_error)?;
    let jobs = rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(internal_error)?;
    Ok(Json(jobs))
}

async fn download(
    State(state): State<AppState>,
    AxumPath((id, name)): AxumPath<(String, String)>,
) -> Result<Response, (StatusCode, Json<ApiError>)> {
    let id = canonical_job_id(&id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "artifact not found"))?;
    let output_dir = state.jobs_dir.join(id);
    let path = artifact_path(&output_dir, &name)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "artifact not found"))?;
    let bytes = tokio::fs::read(&path).await.map_err(internal_error)?;
    let content_type = match path.extension().and_then(|value| value.to_str()) {
        Some("stl") => "model/stl",
        Some("3mf") => "model/3mf",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    };
    let mut response = Response::new(Body::from(bytes));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{name}\""))
            .map_err(internal_error)?,
    );
    Ok(response)
}

fn canonical_job_id(id: &str) -> Option<String> {
    Uuid::parse_str(id)
        .ok()
        .map(|value| value.hyphenated().to_string())
}

fn run_job(
    state: &AppState,
    id: &str,
    spec: &GenerationSpec,
    cancellation: &AtomicBool,
) -> Result<()> {
    let job_started = Instant::now();
    ensure_job_active(cancellation)?;
    update_job(state, id, "running", 10, &[], None)?;
    let phase_started = Instant::now();
    let height_field = elevation::fetch_height_field(spec, &state.map_cache_dir.join("elevation"))?;
    ensure_job_active(cancellation)?;
    info!(
        job_id = %id,
        phase = "elevation",
        elapsed_ms = phase_started.elapsed().as_millis() as u64,
        "generation phase complete"
    );
    update_job(state, id, "running", 40, &[], None)?;
    let surface_field = if spec.color_output.enabled || spec.buildings.enabled {
        let phase_started = Instant::now();
        let field = surface::fetch_surface_field(spec, &height_field, &state.map_cache_dir)?;
        ensure_job_active(cancellation)?;
        info!(
            job_id = %id,
            phase = "surface",
            elapsed_ms = phase_started.elapsed().as_millis() as u64,
            "generation phase complete"
        );
        update_job(state, id, "running", 65, &[], None)?;
        Some(field)
    } else {
        None
    };
    let output_dir = state.jobs_dir.join(id);
    let phase_started = Instant::now();
    let manifest = generate_project_with_fields_cancellable(
        spec,
        &height_field,
        surface_field.as_ref(),
        &output_dir,
        &|| cancellation.load(Ordering::Acquire),
    )?;
    ensure_job_active(cancellation)?;
    info!(
        job_id = %id,
        phase = "mesh",
        elapsed_ms = phase_started.elapsed().as_millis() as u64,
        "generation phase complete"
    );
    update_job(state, id, "complete", 100, &manifest.artifacts, None)?;
    info!(
        job_id = %id,
        elapsed_ms = job_started.elapsed().as_millis() as u64,
        "generation complete"
    );
    Ok(())
}

fn ensure_job_active(cancellation: &AtomicBool) -> Result<()> {
    if cancellation.load(Ordering::Acquire) {
        anyhow::bail!("generation canceled");
    }
    Ok(())
}

fn insert_job(state: &AppState, job: &Job) -> Result<()> {
    let connection = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("database lock failed"))?;
    connection.execute(
        "INSERT INTO jobs
         (id, status, progress, created_at, updated_at, spec_json, artifacts_json, error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            job.id,
            job.status,
            job.progress,
            job.created_at.to_rfc3339(),
            job.updated_at.to_rfc3339(),
            serde_json::to_string(&job.spec)?,
            serde_json::to_string(&job.artifacts)?,
            job.error,
        ],
    )?;
    Ok(())
}

fn update_job(
    state: &AppState,
    id: &str,
    status: &str,
    progress: i64,
    artifacts: &[Artifact],
    error: Option<&str>,
) -> Result<()> {
    let connection = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("database lock failed"))?;
    connection.execute(
        "UPDATE jobs SET status = ?2, progress = ?3, updated_at = ?4,
         artifacts_json = ?5, error = ?6
         WHERE id = ?1 AND status != 'canceled'",
        params![
            id,
            status,
            progress,
            Utc::now().to_rfc3339(),
            serde_json::to_string(artifacts)?,
            error,
        ],
    )?;
    Ok(())
}

fn mark_job_canceled(state: &AppState, id: &str) -> Result<bool> {
    let connection = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("database lock failed"))?;
    let updated = connection.execute(
        "UPDATE jobs
         SET status = 'canceled', updated_at = ?2, artifacts_json = '[]',
             error = NULL
         WHERE id = ?1 AND status IN ('queued', 'running')",
        params![id, Utc::now().to_rfc3339()],
    )?;
    Ok(updated == 1)
}

fn find_job(state: &AppState, id: &str) -> Result<Option<Job>> {
    let connection = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("database lock failed"))?;
    let mut statement = connection.prepare(
        "SELECT id, status, progress, created_at, updated_at, spec_json, artifacts_json, error
         FROM jobs WHERE id = ?1",
    )?;
    let mut rows = statement.query([id])?;
    rows.next()?.map(row_to_job).transpose().map_err(Into::into)
}

fn row_to_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<Job> {
    let created_at: String = row.get(3)?;
    let updated_at: String = row.get(4)?;
    let spec_json: String = row.get(5)?;
    let artifacts_json: String = row.get(6)?;
    Ok(Job {
        id: row.get(0)?,
        status: row.get(1)?,
        progress: row.get(2)?,
        created_at: created_at.parse().map_err(sql_conversion_error)?,
        updated_at: updated_at.parse().map_err(sql_conversion_error)?,
        spec: serde_json::from_str(&spec_json).map_err(sql_conversion_error)?,
        artifacts: serde_json::from_str(&artifacts_json).map_err(sql_conversion_error)?,
        error: row.get(7)?,
    })
}

fn sql_conversion_error(error: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

fn api_error(status: StatusCode, message: impl ToString) -> (StatusCode, Json<ApiError>) {
    (
        status,
        Json(ApiError {
            error: message.to_string(),
        }),
    )
}

fn internal_error(error: impl std::fmt::Display) -> (StatusCode, Json<ApiError>) {
    api_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state() -> AppState {
        let connection = Connection::open_in_memory().unwrap();
        migrate(&connection).unwrap();
        let data_dir =
            std::env::temp_dir().join(format!("toposaic-api-test-{}", std::process::id()));
        AppState {
            db: Arc::new(StdMutex::new(connection)),
            jobs_dir: Arc::new(data_dir.join("jobs")),
            map_cache_dir: Arc::new(data_dir.join("cache")),
            geocoder: Client::new(),
            geocoder_base_url: Arc::new("https://example.invalid".into()),
            last_geocode_request: Arc::new(AsyncMutex::new(Instant::now())),
            active_jobs: Arc::new(StdMutex::new(HashMap::new())),
        }
    }

    #[test]
    fn converts_nominatim_coordinates() {
        let place = PlaceResult::try_from(NominatimPlace {
            display_name: "Mount Rainier, Washington, United States".into(),
            lat: "46.8523".into(),
            lon: "-121.7603".into(),
            category: "natural".into(),
            kind: "peak".into(),
        })
        .unwrap();

        assert_eq!(
            place.display_name,
            "Mount Rainier, Washington, United States"
        );
        assert!((place.latitude - 46.8523).abs() < f64::EPSILON);
        assert!((place.longitude + 121.7603).abs() < f64::EPSILON);
        assert_eq!(place.kind, "peak");
    }

    #[test]
    fn rejects_invalid_nominatim_coordinates() {
        let result = PlaceResult::try_from(NominatimPlace {
            display_name: "Broken".into(),
            lat: "north".into(),
            lon: "west".into(),
            category: "place".into(),
            kind: "unknown".into(),
        });
        assert!(result.is_err());
    }

    #[test]
    fn panic_payload_becomes_a_job_error() {
        assert_eq!(
            panic_message(Box::new("triangulation failed")),
            "mesh generation panicked: triangulation failed"
        );
    }

    #[test]
    fn artifact_downloads_require_uuid_job_directories() {
        assert_eq!(
            canonical_job_id("395481ef-0e39-4d94-9d94-2c39fea86000").as_deref(),
            Some("395481ef-0e39-4d94-9d94-2c39fea86000")
        );
        assert_eq!(canonical_job_id(".."), None);
        assert_eq!(canonical_job_id("../data"), None);
        assert_eq!(canonical_job_id("not-a-job"), None);
    }

    #[test]
    fn canceled_jobs_cannot_return_to_running_or_complete() {
        let state = test_state();
        let now = Utc::now();
        let job = Job {
            id: "395481ef-0e39-4d94-9d94-2c39fea86000".into(),
            status: "running".into(),
            progress: 40,
            created_at: now,
            updated_at: now,
            spec: GenerationSpec::default(),
            artifacts: Vec::new(),
            error: None,
        };
        insert_job(&state, &job).unwrap();

        assert!(mark_job_canceled(&state, &job.id).unwrap());
        update_job(&state, &job.id, "complete", 100, &[], None).unwrap();

        let canceled = find_job(&state, &job.id).unwrap().unwrap();
        assert_eq!(canceled.status, "canceled");
        assert_eq!(canceled.progress, 40);
        assert!(canceled.artifacts.is_empty());
        assert!(!mark_job_canceled(&state, &job.id).unwrap());
    }
}
