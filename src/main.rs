// Same reason as the one in lib.rs, and it has to be said again here: the
// binary is its own crate, and it lays out the same views to render them
// server-side.
#![recursion_limit = "256"]

/// Serves the stored receipt photo for the review screen.
///
/// Not a server function: this returns raw image bytes, not a serialized value.
/// The path is read from the database and the filename was generated at upload,
/// so nothing user-controlled reaches the filesystem.
#[cfg(feature = "ssr")]
async fn receipt_image(
    axum::Extension(state): axum::Extension<tally_ho::server::state::AppState>,
    axum::extract::Path(id): axum::extract::Path<uuid::Uuid>,
) -> axum::response::Response {
    use axum::http::{StatusCode, header};
    use axum::response::IntoResponse;
    use tally_ho::server::models::Receipt;

    let mut db = state.db.clone();

    let Ok(receipt) = Receipt::get_by_id(&mut db, &id).await else {
        return (StatusCode::NOT_FOUND, "no such receipt").into_response();
    };
    let Ok(bytes) = state.store.read(&receipt.image_path).await else {
        return (StatusCode::NOT_FOUND, "image missing").into_response();
    };

    let content_type = match receipt.image_path.rsplit('.').next() {
        Some("webp") => "image/webp",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        _ => "application/octet-stream",
    };

    (
        [
            (header::CONTENT_TYPE, content_type),
            // The filename is a uuid and the bytes are never rewritten, so this
            // is safe to cache hard. `private` because receipts are personal.
            (
                header::CACHE_CONTROL,
                "private, max-age=31536000, immutable",
            ),
        ],
        bytes,
    )
        .into_response()
}

/// Exports a reconciled statement as CSV.
///
/// A plain route rather than a server function: this way it is an ordinary link,
/// which on a phone hands the file straight to the OS share sheet with a real
/// filename. A server function would return the CSV as a string that the client
/// then has to turn into a download itself.
#[cfg(feature = "ssr")]
async fn export_csv(
    axum::Extension(state): axum::Extension<tally_ho::server::state::AppState>,
    axum::extract::Path(id): axum::extract::Path<uuid::Uuid>,
) -> axum::response::Response {
    use axum::http::{StatusCode, header};
    use axum::response::IntoResponse;

    let mut db = state.db.clone();
    let statement = match tally_ho::server::queries::statements::load(&mut db, id).await {
        Ok(statement) => statement,
        Err(e) => {
            return (StatusCode::NOT_FOUND, format!("no such statement: {e}")).into_response();
        }
    };

    let csv = tally_ho::shared::export::statement_to_csv(&statement);
    let name = format!(
        "tally-ho-{}-to-{}.csv",
        statement.begins_on, statement.ends_on
    );

    (
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
            // Every figure in here changes as charges get resolved, so this must
            // never be served from a cache.
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        csv,
    )
        .into_response()
}

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    use axum::Router;
    use axum::routing::get;
    use leptos::logging::log;
    use leptos::prelude::*;
    use leptos_axum::{LeptosRoutes, generate_route_list};
    use tally_ho::frontend::*;
    use tally_ho::server::state::AppState;

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tally_ho=debug".into()),
        )
        .init();

    let state = AppState::from_env()
        .await
        .expect("could not initialize app state");
    log!("data dir: {}", state.store.root().display());

    let conf = get_configuration(None).unwrap();
    let addr = conf.leptos_options.site_addr;
    let leptos_options = conf.leptos_options;
    let routes = generate_route_list(App);

    let app = Router::new()
        .route("/receipt-image/{id}", get(receipt_image))
        .route("/statement/{id}/export.csv", get(export_csv))
        // `_with_context` is what makes `expect_context::<AppState>()` work
        // inside server functions.
        .leptos_routes_with_context(
            &leptos_options,
            routes,
            {
                let state = state.clone();
                move || provide_context(state.clone())
            },
            {
                let leptos_options = leptos_options.clone();
                move || shell(leptos_options.clone())
            },
        )
        .fallback(leptos_axum::file_and_error_handler(shell))
        // Makes AppState available to the plain axum handler above; the Leptos
        // routes get it through context instead.
        .layer(axum::Extension(state))
        .with_state(leptos_options);

    log!("listening on http://{}", &addr);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();
}

#[cfg(not(feature = "ssr"))]
pub fn main() {
    // The client entrypoint is `hydrate()` in lib.rs; this exists only so the
    // bin target still compiles when building the wasm lib.
}
