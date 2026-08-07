//! The UI. Rendered on the server first and hydrated in the browser, so all of
//! it compiles for both targets.
//!
//! A module per screen — capture, receipts, review, period — and the rest is
//! bits more than one of them needs.

mod capture;
mod money;
mod period;
mod photo;
mod receipts;
mod review;
mod ui;

use leptos::prelude::*;
use leptos_meta::{MetaTags, Stylesheet, Title, provide_meta_context};
use leptos_router::components::{A, Route, Router, Routes};
use leptos_router::path;

use capture::CapturePage;
use period::PeriodPage;
use receipts::ReceiptListPage;
use review::ReviewPage;

pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                // viewport-fit=cover goes with the px-safe/pb-safe utilities, so
                // content clears the iOS notch and home indicator once installed.
                <meta
                    name="viewport"
                    content="width=device-width, initial-scale=1, viewport-fit=cover"
                />
                <meta name="theme-color" content="#16181a" />
                // Manifest plus HTTPS is all it takes to be installable — Chrome
                // dropped the service-worker requirement for menu install in 108.
                <link rel="manifest" href="/manifest.webmanifest" />
                <link rel="icon" type="image/png" href="/favicon.png" />
                <AutoReload options=options.clone() />
                <HydrationScripts options />
                <MetaTags />
            </head>
            <body>
                <App />
            </body>
        </html>
    }
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    view! {
        <Stylesheet id="leptos" href="/pkg/tally_ho.css" />
        <Title text="tally-ho" />

        <Router>
            // Nav first so it can stick to the top on desktop. On a phone it is
            // `fixed`, so DOM order doesn't affect where it lands.
            <NavBar />
            <main class="px-safe mx-auto w-full max-w-4xl pt-4 pb-28 md:pb-12">
                <Routes fallback=|| view! { <p>"Not found."</p> }>
                    <Route path=path!("/") view=CapturePage />
                    <Route path=path!("/receipts") view=ReceiptListPage />
                    <Route path=path!("/receipt/:id") view=ReviewPage />
                    <Route path=path!("/period") view=PeriodPage />
                </Routes>
            </main>
        </Router>
    }
}

/// Thumb-reachable bottom bar on a phone, ordinary top bar on a desktop.
#[component]
fn NavBar() -> impl IntoView {
    // min-h-11 is 44px, the smallest comfortable thumb target. Tabs split the width
    // on a phone and shrink to their labels on desktop.
    let link = "flex min-h-11 flex-1 items-center justify-center text-sm text-muted \
                no-underline active:bg-edge aria-[current=page]:text-paper \
                md:flex-none md:px-4 md:hover:text-paper";
    let bar = "pb-safe fixed inset-x-0 bottom-0 z-10 flex border-t border-edge bg-surface \
               md:sticky md:top-0 md:bottom-auto md:border-t-0 md:border-b md:pb-0";
    view! {
        <nav class=bar>
            // Same max-width as <main>, so the tabs line up with the content.
            <div class="px-safe mx-auto flex w-full max-w-4xl">
                <span class="mr-auto hidden items-center pr-4 font-semibold md:flex">"tally-ho"</span>
                <A href="/" attr:class=link>
                    "Capture"
                </A>
                <A href="/receipts" attr:class=link>
                    "Receipts"
                </A>
                <A href="/period" attr:class=link>
                    "Period"
                </A>
            </div>
        </nav>
    }
}
