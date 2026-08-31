use crate::prelude::*;

pub fn stylesheet_link() -> Markup {
    html! {
        link rel="stylesheet" href="/res/styles.css";
    }
}

pub fn scripts() -> Markup {
    html! {
        script src="/res/htmx.min.js" {}
        script src="/res/idiomorph.min.js" {}
        script src="/res/idiomorph-ext.min.js" {}
    }
}

pub fn render(active_page: &str) -> Markup {
    let nav_item = |key: &str, href: &str, label: &str| {
        html! {
            a class=(if active_page == key { "header-nav-item active" } else { "header-nav-item" })
                href=(href) { (label) }
        }
    };

    html! {
        header class="header" {
            div class="header-title" { a href="/" { "watchtower" } }
            nav class="header-nav" {
                (nav_item("repos", "/", "Repos"))
                (nav_item("metrics", "/api/metrics", "Metrics"))
            }
        }
    }
}

/// Common page shell: header chrome, stylesheet, htmx + idiomorph.
pub fn page(title: &str, active_page: &str, page_class: &str, content: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="UTF-8";
                meta name="viewport" content="width=device-width, initial-scale=1.0";
                title { (title) " · watchtower" }
                (stylesheet_link())
                (scripts())
            }
            body class=(format!("{}-page", page_class)) hx-ext="morph" {
                (render(active_page))
                div class="content" { (content) }
            }
        }
    }
}

pub fn html_response(markup: Markup) -> HttpResponse {
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(markup.into_string())
}
