// use actix_web::web;
use maud::{Markup, html};

use crate::prelude::*;

fn render_event_row() -> Markup {
    html! {
        details
            class="webhook-event"
            hx-preserve="true"
            hx-trigger="toggle once"
            hx-target="find .webhook-payload"
            hx-swap="innerHTML"
        {
            summary class="webhook-event-summary" {
            }
            div class="webhook-payload" { "Loading…" }
        }
    }
}

#[get("/webhooks/recent")]
pub async fn webhook_recent_list() -> impl Responder {
    let events: Vec<()> = vec![];
    let markup = html! {
        @if events.is_empty() {
            div class="empty-state" {
                h2 { "No webhook events yet" }
                p { "Events will appear here as they are received." }
            }
        } @else {
            @for __e in &events {
                (render_event_row())
            }
        }
    };

    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(markup.into_string())
}

// #[get("/webhooks/recent/{id}")]
// pub async fn webhook_recent_payload(path: web::Path<u64>) -> impl Responder {
//     let id = path.into_inner();
//     let markup = match recent::get(id) {
//         Some(e) => {
//             let pretty =
//                 serde_json::to_string_pretty(&e.payload).unwrap_or_else(|_| e.payload.to_string());
//             html! { pre class="webhook-payload-json" { (pretty) } }
//         }
//         None => html! {
//             div class="webhook-payload-missing" {
//                 "This event is no longer retained (it has been evicted from the buffer)."
//             }
//         },
//     };

//     HttpResponse::Ok()
//         .content_type("text/html; charset=utf-8")
//         .body(markup.into_string())
// }
