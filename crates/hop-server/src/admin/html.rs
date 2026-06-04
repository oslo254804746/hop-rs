use hop_core::{Asset, AuthorizedKey, Credential, KnownHost, Session};
use maud::{html, Markup, DOCTYPE};

use super::{
    i18n::{L10n, Locale},
    transfer::ImportSummary,
};

pub fn layout(title: &str, active: &str, t: &L10n, body_content: Markup) -> Markup {
    let alternate = t.locale.alternate();
    let language_href = language_switch_href(alternate, active);
    html! {
        (DOCTYPE)
        html lang=(t.locale.code()) {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) " - " (t.app_title) }
                style {
                    r#"
                    :root {
                        color-scheme: light;
                        --ink: #171a1f;
                        --ink-soft: #4d5562;
                        --muted: #737b88;
                        --line: #dfe4e0;
                        --surface: #ffffff;
                        --surface-soft: #f6f8f5;
                        --field: #fbfcfb;
                        --accent: #0f766e;
                        --accent-strong: #0b5f59;
                        --accent-soft: #d9efec;
                        --danger: #b42318;
                        --danger-soft: #fde8e6;
                        --shadow: 0 18px 50px rgba(23, 26, 31, 0.08);
                    }

                    * { box-sizing: border-box; }

                    body.admin-shell {
                        margin: 0;
                        min-height: 100vh;
                        background:
                            linear-gradient(135deg, rgba(15, 118, 110, 0.08), transparent 34%),
                            var(--surface-soft);
                        color: var(--ink);
                        font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
                        letter-spacing: 0;
                    }

                    a { color: inherit; }

                    .app-frame {
                        display: grid;
                        grid-template-columns: 278px minmax(0, 1fr);
                        min-height: 100vh;
                    }

                    .sidebar {
                        position: sticky;
                        top: 0;
                        height: 100svh;
                        padding: 24px 18px;
                        background: #171a1f;
                        color: #eef4f1;
                        display: flex;
                        flex-direction: column;
                        gap: 28px;
                    }

                    .brand {
                        display: grid;
                        grid-template-columns: 44px minmax(0, 1fr);
                        gap: 12px;
                        align-items: center;
                        padding: 0 6px;
                    }

                    .brand-mark {
                        width: 44px;
                        height: 44px;
                        border-radius: 8px;
                        display: grid;
                        place-items: center;
                        background: var(--accent);
                        color: white;
                        font-weight: 800;
                        font-size: 1.1rem;
                    }

                    .brand strong { display: block; font-size: 1.04rem; }
                    .brand span { color: #aab4b0; font-size: 0.82rem; }

                    .nav {
                        display: grid;
                        gap: 5px;
                    }

                    .nav-link {
                        min-height: 40px;
                        display: flex;
                        align-items: center;
                        gap: 10px;
                        padding: 9px 10px;
                        border-radius: 8px;
                        color: #c9d3cf;
                        text-decoration: none;
                        font-weight: 650;
                        font-size: 0.92rem;
                        transition: background 160ms ease, color 160ms ease, transform 160ms ease;
                    }

                    .nav-link::before {
                        content: "";
                        width: 8px;
                        height: 8px;
                        border-radius: 999px;
                        background: #66716d;
                    }

                    .nav-link:hover {
                        background: rgba(255, 255, 255, 0.07);
                        color: white;
                        transform: translateX(2px);
                    }

                    .nav-link.active {
                        background: rgba(15, 118, 110, 0.22);
                        color: white;
                    }

                    .nav-link.active::before { background: #62d6cb; }

                    .sidebar-footer {
                        margin-top: auto;
                        padding: 14px;
                        border: 1px solid rgba(255, 255, 255, 0.1);
                        border-radius: 8px;
                        background: rgba(255, 255, 255, 0.05);
                        color: #dce5e1;
                        font-size: 0.86rem;
                    }

                    .sidebar-footer small {
                        display: block;
                        margin-top: 4px;
                        color: #9ca8a4;
                        line-height: 1.45;
                    }

                    .status-dot {
                        width: 8px;
                        height: 8px;
                        display: inline-block;
                        margin-right: 8px;
                        border-radius: 999px;
                        background: #62d6cb;
                        box-shadow: 0 0 0 4px rgba(98, 214, 203, 0.14);
                    }

                    .language-switch {
                        display: flex;
                        align-items: center;
                        justify-content: space-between;
                        gap: 10px;
                        margin-top: 12px;
                        padding-top: 12px;
                        border-top: 1px solid rgba(255, 255, 255, 0.1);
                    }

                    .language-switch a {
                        color: #62d6cb;
                        font-weight: 780;
                        text-decoration: none;
                    }

                    .content-shell { min-width: 0; }

                    .topbar {
                        position: sticky;
                        top: 0;
                        z-index: 10;
                        display: flex;
                        align-items: flex-start;
                        justify-content: space-between;
                        gap: 20px;
                        padding: 28px 38px 18px;
                        border-bottom: 1px solid rgba(223, 228, 224, 0.85);
                        background: rgba(246, 248, 245, 0.9);
                        backdrop-filter: blur(14px);
                    }

                    .eyebrow {
                        margin: 0 0 6px;
                        color: var(--accent);
                        font-size: 0.76rem;
                        font-weight: 800;
                        text-transform: uppercase;
                    }

                    .topbar h1 {
                        margin: 0;
                        font-size: clamp(1.7rem, 2.6vw, 2.35rem);
                        line-height: 1.05;
                    }

                    .workspace {
                        width: min(1240px, 100%);
                        padding: 28px 38px 58px;
                    }

                    .page-intro {
                        display: grid;
                        gap: 8px;
                        margin-bottom: 22px;
                    }

                    .page-intro h2 {
                        margin: 0;
                        font-size: 1.28rem;
                    }

                    .page-intro p {
                        margin: 0;
                        color: var(--ink-soft);
                        max-width: 760px;
                        line-height: 1.6;
                    }

                    .panel {
                        margin: 0 0 22px;
                        padding: 22px;
                        border: 1px solid var(--line);
                        border-radius: 8px;
                        background: rgba(255, 255, 255, 0.88);
                        box-shadow: var(--shadow);
                    }

                    .panel-header {
                        display: flex;
                        align-items: flex-start;
                        justify-content: space-between;
                        gap: 18px;
                        margin-bottom: 18px;
                    }

                    .panel-header h2 {
                        margin: 0;
                        font-size: 1.05rem;
                    }

                    .panel-header p {
                        margin: 5px 0 0;
                        color: var(--muted);
                        line-height: 1.5;
                    }

                    .grid {
                        display: grid;
                        grid-template-columns: repeat(auto-fit, minmax(230px, 1fr));
                        gap: 14px;
                    }

                    .field-wide { grid-column: 1 / -1; }

                    .button-row {
                        display: flex;
                        align-items: center;
                        flex-wrap: wrap;
                        gap: 10px;
                        margin-top: 16px;
                    }

                    .metric-grid {
                        display: grid;
                        grid-template-columns: repeat(auto-fit, minmax(190px, 1fr));
                        gap: 14px;
                        margin-bottom: 22px;
                    }

                    .metric {
                        min-height: 132px;
                        padding: 18px;
                        border: 1px solid var(--line);
                        border-radius: 8px;
                        background: var(--surface);
                        display: flex;
                        flex-direction: column;
                        justify-content: space-between;
                        box-shadow: var(--shadow);
                    }

                    .metric-label {
                        color: var(--muted);
                        font-size: 0.84rem;
                        font-weight: 700;
                    }

                    .metric-value {
                        font-size: 2.35rem;
                        line-height: 1;
                        font-weight: 850;
                    }

                    .metric-note {
                        color: var(--ink-soft);
                        font-size: 0.84rem;
                    }

                    label.field {
                        display: grid;
                        gap: 7px;
                        color: var(--ink-soft);
                        font-size: 0.84rem;
                        font-weight: 720;
                    }

                    input, select, textarea {
                        width: 100%;
                        min-height: 42px;
                        padding: 10px 11px;
                        border: 1px solid #cfd7d2;
                        border-radius: 7px;
                        background: var(--field);
                        color: var(--ink);
                        font: inherit;
                        font-size: 0.94rem;
                        transition: border-color 140ms ease, box-shadow 140ms ease, background 140ms ease;
                    }

                    textarea {
                        min-height: 112px;
                        resize: vertical;
                        line-height: 1.45;
                    }

                    input:focus, select:focus, textarea:focus {
                        outline: 0;
                        border-color: var(--accent);
                        background: white;
                        box-shadow: 0 0 0 4px rgba(15, 118, 110, 0.13);
                    }

                    input[type="hidden"] { display: none; }

                    button, .button, .ghost-button {
                        min-height: 38px;
                        display: inline-flex;
                        align-items: center;
                        justify-content: center;
                        gap: 8px;
                        border: 0;
                        border-radius: 7px;
                        padding: 9px 13px;
                        font: inherit;
                        font-weight: 760;
                        font-size: 0.9rem;
                        text-decoration: none;
                        cursor: pointer;
                        transition: transform 140ms ease, background 140ms ease, color 140ms ease, box-shadow 140ms ease;
                    }

                    button, .button {
                        background: var(--accent);
                        color: white;
                        box-shadow: 0 9px 22px rgba(15, 118, 110, 0.2);
                    }

                    button:hover, .button:hover {
                        background: var(--accent-strong);
                        transform: translateY(-1px);
                    }

                    .ghost-button {
                        border: 1px solid var(--line);
                        background: white;
                        color: var(--ink);
                    }

                    .ghost-button:hover {
                        border-color: #c5cec8;
                        transform: translateY(-1px);
                    }

                    button.danger, .danger {
                        background: var(--danger);
                        color: white;
                        box-shadow: 0 9px 22px rgba(180, 35, 24, 0.15);
                    }

                    button.danger:hover, .danger:hover { background: #961f16; }

                    .muted, .fine-print {
                        color: var(--muted);
                        line-height: 1.55;
                    }

                    .error-message {
                        margin: 0 0 16px;
                        padding: 10px 12px;
                        border-radius: 7px;
                        background: var(--danger-soft);
                        color: var(--danger);
                        font-weight: 720;
                    }

                    .fine-print {
                        margin: 14px 0 0;
                        font-size: 0.86rem;
                    }

                    .table-wrap {
                        overflow-x: auto;
                        border: 1px solid var(--line);
                        border-radius: 8px;
                        background: white;
                    }

                    table.data-table {
                        width: 100%;
                        min-width: 760px;
                        border-collapse: collapse;
                    }

                    .data-table th,
                    .data-table td {
                        padding: 13px 14px;
                        border-bottom: 1px solid var(--line);
                        text-align: left;
                        vertical-align: top;
                    }

                    .data-table th {
                        color: var(--muted);
                        background: #f8faf8;
                        font-size: 0.76rem;
                        font-weight: 800;
                        text-transform: uppercase;
                    }

                    .data-table tr:last-child td { border-bottom: 0; }
                    .data-table tbody tr { transition: background 140ms ease; }
                    .data-table tbody tr:hover { background: #f4faf8; }

                    .primary-cell {
                        display: grid;
                        gap: 4px;
                        font-weight: 760;
                    }

                    .subtle {
                        color: var(--muted);
                        font-size: 0.84rem;
                        font-weight: 500;
                    }

                    .mono {
                        font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, "Liberation Mono", monospace;
                        font-size: 0.84rem;
                        line-height: 1.45;
                        word-break: break-all;
                    }

                    .tag-list, .secret-list, .action-row {
                        display: flex;
                        align-items: center;
                        flex-wrap: wrap;
                        gap: 7px;
                    }

                    .filter-row {
                        display: flex;
                        align-items: center;
                        flex-wrap: wrap;
                        gap: 8px;
                    }

                    .checkbox-cell {
                        width: 42px;
                        text-align: center;
                    }

                    .checkbox-cell input {
                        width: 18px;
                        min-height: 18px;
                    }

                    .import-summary {
                        display: grid;
                        gap: 8px;
                    }

                    .tag, .status-pill {
                        display: inline-flex;
                        align-items: center;
                        min-height: 24px;
                        border-radius: 999px;
                        padding: 3px 9px;
                        font-size: 0.78rem;
                        font-weight: 760;
                        white-space: nowrap;
                    }

                    .tag {
                        background: #edf4ef;
                        color: #315849;
                    }

                    .status-pill {
                        background: var(--accent-soft);
                        color: var(--accent-strong);
                    }

                    .status-pill.neutral {
                        background: #edf0f2;
                        color: #55606d;
                    }

                    .status-pill.danger {
                        background: var(--danger-soft);
                        color: var(--danger);
                        box-shadow: none;
                    }

                    .action-row form { margin: 0; }

                    .empty-row td {
                        padding: 28px 14px;
                        color: var(--muted);
                        text-align: center;
                    }

                    .login-wrap {
                        max-width: 440px;
                        margin: 8vh auto 0;
                    }

                    pre {
                        white-space: pre-wrap;
                        border: 1px solid var(--line);
                        border-radius: 8px;
                        background: white;
                        padding: 14px;
                    }

                    @media (max-width: 900px) {
                        .app-frame { grid-template-columns: 1fr; }
                        .sidebar {
                            position: static;
                            height: auto;
                            padding: 16px;
                            gap: 16px;
                        }
                        .nav {
                            grid-template-columns: repeat(2, minmax(0, 1fr));
                        }
                        .sidebar-footer { display: none; }
                        .topbar {
                            position: static;
                            padding: 22px 20px 16px;
                        }
                        .workspace {
                            padding: 22px 20px 44px;
                        }
                    }

                    @media (max-width: 560px) {
                        .brand { grid-template-columns: 38px minmax(0, 1fr); }
                        .brand-mark { width: 38px; height: 38px; }
                        .nav { grid-template-columns: 1fr; }
                        .topbar {
                            flex-direction: column;
                            align-items: stretch;
                        }
                        .ghost-button { width: 100%; }
                        .panel { padding: 17px; }
                        .panel-header {
                            flex-direction: column;
                        }
                    }
                    "#
                }
            }
            body class="admin-shell" {
                div.app-frame {
                    aside.sidebar {
                        div.brand {
                            div.brand-mark { "H" }
                            div {
                                strong { "Hop" }
                                span { (t.admin_console) }
                            }
                        }
                        nav.nav aria-label=(t.nav_primary) {
                            (nav_link("/", t.nav_overview, active == "overview"))
                            (nav_link("/assets", t.nav_assets, active == "assets"))
                            (nav_link("/credentials", t.nav_credentials, active == "credentials"))
                            (nav_link("/keys", t.nav_keys, active == "keys"))
                            (nav_link("/known-hosts", t.nav_known_hosts, active == "known-hosts"))
                            (nav_link("/sessions", t.nav_sessions, active == "sessions"))
                            (nav_link("/import", t.nav_import_export, active == "import"))
                        }
                        div.sidebar-footer {
                            span.status-dot {}
                            (t.loopback_admin)
                            small { (t.loopback_note) }
                            div.language-switch {
                                span { (t.language_label) ": " (t.locale.label()) }
                                a href=(language_href) { (t.switch_language_to) " " (alternate.label()) }
                            }
                        }
                    }
                    div.content-shell {
                        header.topbar {
                            div {
                                p.eyebrow { (t.admin_web) }
                                h1 { (title) }
                            }
                            @if active != "login" {
                                a.ghost-button href="/logout" { (t.logout) }
                            }
                        }
                        main.workspace { (body_content) }
                    }
                }
            }
        }
    }
}

pub fn login(t: &L10n, error: Option<&str>) -> Markup {
    layout(
        t.login_title,
        "login",
        t,
        html! {
            div.login-wrap {
                section.panel {
                    div.panel-header {
                        div {
                            h2 { (t.login_heading) }
                            p { (t.login_intro) }
                        }
                    }
                    @if let Some(error) = error {
                        p.error-message { (error) }
                    }
                    form method="post" action="/login" {
                        label.field {
                            (t.login_password)
                            input type="password" name="password" required;
                        }
                        div.button-row {
                            button type="submit" { (t.login_button) }
                        }
                    }
                }
            }
        },
    )
}

pub fn overview(
    t: &L10n,
    asset_count: usize,
    credential_count: usize,
    key_count: usize,
    session_count: usize,
) -> Markup {
    layout(
        t.overview_title,
        "overview",
        t,
        html! {
            div.page-intro {
                h2 { (t.overview_heading) }
                p { (t.overview_intro) }
            }
            div.metric-grid {
                div.metric {
                    span.metric-label { (t.overview_assets_label) }
                    strong.metric-value { (asset_count) }
                    span.metric-note { (t.overview_assets_note) }
                }
                div.metric {
                    span.metric-label { (t.overview_credentials_label) }
                    strong.metric-value { (credential_count) }
                    span.metric-note { (t.overview_credentials_note) }
                }
                div.metric {
                    span.metric-label { (t.overview_keys_label) }
                    strong.metric-value { (key_count) }
                    span.metric-note { (t.overview_keys_note) }
                }
                div.metric {
                    span.metric-label { (t.overview_sessions_label) }
                    strong.metric-value { (session_count) }
                    span.metric-note { (t.overview_sessions_note) }
                }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.overview_scope_heading) }
                        p { (t.overview_scope_intro) }
                    }
                }
                p.fine-print { (t.overview_scope_note) }
            }
        },
    )
}

pub fn assets(
    t: &L10n,
    items: &[Asset],
    credentials: &[Credential],
    csrf_token: &str,
    selected_tag: Option<&str>,
    all_tags: &[String],
) -> Markup {
    layout(
        t.assets_title,
        "assets",
        t,
        html! {
            div.page-intro {
                h2 { (t.assets_heading) }
                p { (t.assets_intro) }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.assets_filter_heading) }
                        p { (t.assets_filter_intro) }
                    }
                }
                div.filter-row {
                    a class=(if selected_tag.is_none() { "button" } else { "ghost-button" }) href="/assets" {
                        (t.assets_filter_all)
                    }
                    @for tag in all_tags {
                        a class=(if selected_tag == Some(tag.as_str()) { "button" } else { "ghost-button" })
                          href=(format!("/assets?tag={}", url_query_value(tag))) {
                            (tag)
                        }
                    }
                }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.assets_export_heading) }
                        p { (t.assets_export_intro) }
                    }
                }
                div.button-row {
                    a.button href="/assets/export?format=csv" { (t.export_csv) }
                    a.button href="/assets/export?format=json" { (t.export_json) }
                    a.ghost-button href="/import" { (t.import_open) }
                }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.assets_add_heading) }
                        p { (t.assets_add_intro) }
                    }
                }
                form method="post" action="/assets" {
                    (csrf_field(csrf_token))
                    div.grid {
                        label.field {
                            (t.field_name)
                            input name="name" required;
                        }
                        label.field {
                            (t.field_hostname)
                            input name="hostname" required;
                        }
                        label.field {
                            (t.field_port)
                            input name="port" type="number" value="22" required;
                        }
                        label.field {
                            (t.field_tags)
                            input name="tags" placeholder="prod, web" list="asset-tags-list";
                        }
                        label.field {
                            (t.field_credential)
                            select name="credential_id" {
                                option value="" { (t.proxy_only) }
                                @for credential in credentials {
                                    option value=(credential.id) { (credential.name) " (" (credential.username) ")" }
                                }
                            }
                        }
                        label.field.field-wide {
                            (t.field_description)
                            textarea name="description" {}
                        }
                    }
                    datalist id="asset-tags-list" {
                        @for tag in all_tags {
                            option value=(tag) {}
                        }
                    }
                    div.button-row {
                        button type="submit" { (t.save_asset) }
                    }
                }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.assets_existing_heading) }
                        p { (t.assets_existing_intro) }
                    }
                }
                form method="post" action="/assets/bulk-tags" {
                    (csrf_field(csrf_token))
                    div.table-wrap {
                        table.data-table {
                            thead {
                                tr {
                                    th.checkbox-cell {}
                                    th { (t.field_name) }
                                    th { (t.target_column) }
                                    th { (t.field_tags) }
                                    th { (t.field_credential) }
                                    th { (t.field_action) }
                                }
                            }
                            tbody {
                                @if items.is_empty() {
                                    tr.empty-row { td colspan="6" { (t.no_assets) } }
                                }
                                @for asset in items {
                                    tr {
                                        td.checkbox-cell {
                                            input type="checkbox" name="asset_ids" value=(asset.id);
                                        }
                                        td {
                                            div.primary-cell {
                                                (asset.name)
                                                @if let Some(description) = &asset.description {
                                                    span.subtle { (description) }
                                                }
                                            }
                                        }
                                        td.mono { (asset.hostname) ":" (asset.port) }
                                        td {
                                            div.tag-list {
                                                @if asset.tags.is_empty() {
                                                    span.status-pill.neutral { (t.untagged) }
                                                }
                                                @for tag in &asset.tags {
                                                    a.tag href=(format!("/assets?tag={}", url_query_value(tag))) { (tag) }
                                                }
                                            }
                                        }
                                        td {
                                            @if let Some(credential_id) = &asset.credential_id {
                                                span.status-pill { (credential_id) }
                                            } @else {
                                                span.status-pill.neutral { (t.proxy_only) }
                                            }
                                        }
                                        td {
                                            div.action-row {
                                                a class="button" href=(format!("/assets/{}/edit", asset.id)) { (t.edit) }
                                                button class="danger" type="submit" formaction=(format!("/assets/{}/delete", asset.id)) { (t.delete) }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    div.panel-header style="margin-top:18px;" {
                        div {
                            h2 { (t.assets_bulk_heading) }
                            p { (t.assets_bulk_intro) }
                        }
                    }
                    div.grid {
                        label.field {
                            (t.assets_bulk_tags_label)
                            input name="tags" placeholder="prod, web" list="asset-tags-list";
                        }
                    }
                    div.button-row {
                        button type="submit" { (t.assets_bulk_apply) }
                    }
                }
            }
        },
    )
}

pub fn edit_asset(
    t: &L10n,
    asset: &Asset,
    credentials: &[Credential],
    csrf_token: &str,
    all_tags: &[String],
) -> Markup {
    layout(
        t.edit_asset_title,
        "assets",
        t,
        html! {
            div.page-intro {
                h2 { (asset.name) }
                p { (t.edit_asset_intro) }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.asset_details_heading) }
                        p { (t.asset_details_intro) }
                    }
                }
                form method="post" action=(format!("/assets/{}", asset.id)) {
                    (csrf_field(csrf_token))
                    div.grid {
                        label.field {
                            (t.field_name)
                            input name="name" value=(asset.name) required;
                        }
                        label.field {
                            (t.field_hostname)
                            input name="hostname" value=(asset.hostname) required;
                        }
                        label.field {
                            (t.field_port)
                            input name="port" type="number" value=(asset.port) required;
                        }
                        label.field {
                            (t.field_tags)
                            input name="tags" value=(asset.tags.join(",")) placeholder="prod, web" list="asset-tags-list";
                        }
                        label.field {
                            (t.field_credential)
                            select name="credential_id" {
                                option value="" selected[asset.credential_id.is_none()] { (t.proxy_only) }
                                @for credential in credentials {
                                    option value=(credential.id) selected[asset.credential_id.as_deref() == Some(credential.id.as_str())] {
                                        (credential.name) " (" (credential.username) ")"
                                    }
                                }
                            }
                        }
                        label.field.field-wide {
                            (t.field_description)
                            textarea name="description" { (asset.description.as_deref().unwrap_or("")) }
                        }
                    }
                    datalist id="asset-tags-list" {
                        @for tag in all_tags {
                            option value=(tag) {}
                        }
                    }
                    div.button-row {
                        button type="submit" { (t.save_changes) }
                        a.ghost-button href="/assets" { (t.back_to_assets) }
                    }
                }
            }
        },
    )
}

pub fn credentials(t: &L10n, items: &[Credential], csrf_token: &str) -> Markup {
    layout(
        t.credentials_title,
        "credentials",
        t,
        html! {
            div.page-intro {
                h2 { (t.credentials_heading) }
                p { (t.credentials_intro) }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.credentials_export_heading) }
                        p { (t.credentials_export_intro) }
                    }
                }
                div.button-row {
                    a.button href="/credentials/export?format=csv" { (t.export_csv) }
                    a.button href="/credentials/export?format=json" { (t.export_json) }
                    a.ghost-button href="/import" { (t.import_open) }
                }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.credentials_add_heading) }
                        p { (t.credentials_add_intro) }
                    }
                }
                form method="post" action="/credentials" {
                    (csrf_field(csrf_token))
                    div.grid {
                        label.field {
                            (t.field_name)
                            input name="name" required;
                        }
                        label.field {
                            (t.field_username)
                            input name="username" required;
                        }
                        label.field {
                            (t.field_auth_type)
                            select name="auth_type" {
                                option value="password" { "password" }
                                option value="key" { "key" }
                                option value="key+passphrase" { "key+passphrase" }
                            }
                        }
                        label.field {
                            (t.field_password)
                            input type="password" name="password";
                        }
                        label.field {
                            (t.field_passphrase)
                            input type="password" name="passphrase";
                        }
                        label.field.field-wide {
                            (t.field_private_key)
                            textarea name="private_key" rows="8" {}
                        }
                    }
                    div.button-row {
                        button type="submit" { (t.save_credential) }
                    }
                    p.fine-print { (t.secret_storage_note) }
                }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.credentials_existing_heading) }
                        p { (t.credentials_existing_intro) }
                    }
                }
                div.table-wrap {
                    table.data-table {
                        thead {
                            tr { th { (t.field_name) } th { (t.field_username) } th { (t.field_auth_type) } th { (t.secrets_label) } th { (t.field_action) } }
                        }
                        tbody {
                            @if items.is_empty() {
                                tr.empty-row { td colspan="5" { (t.no_credentials) } }
                            }
                            @for credential in items {
                                tr {
                                    td {
                                        div.primary-cell {
                                            (credential.name)
                                            span.subtle.mono { (credential.id) }
                                        }
                                    }
                                    td { (credential.username) }
                                    td { span.status-pill { (credential.auth_type) } }
                                    td {
                                        div.secret-list {
                                            @if credential.password_enc.is_none() && credential.private_key_enc.is_none() && credential.passphrase_enc.is_none() {
                                                span.status-pill.neutral { (t.none) }
                                            }
                                            @if credential.password_enc.is_some() {
                                                span.tag { "password" }
                                            }
                                            @if credential.private_key_enc.is_some() {
                                                span.tag { "private key" }
                                            }
                                            @if credential.passphrase_enc.is_some() {
                                                span.tag { "passphrase" }
                                            }
                                        }
                                    }
                                    td {
                                        div.action-row {
                                            a class="button" href=(format!("/credentials/{}/edit", credential.id)) { (t.edit) }
                                            form method="post" action=(format!("/credentials/{}/delete", credential.id)) {
                                                (csrf_field(csrf_token))
                                                button class="danger" type="submit" { (t.delete) }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
    )
}

pub fn edit_credential(t: &L10n, credential: &Credential, csrf_token: &str) -> Markup {
    layout(
        t.edit_credential_title,
        "credentials",
        t,
        html! {
            div.page-intro {
                h2 { (credential.name) }
                p { (t.edit_credential_intro) }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.credential_details_heading) }
                        p { (t.credential_details_intro) }
                    }
                }
                form method="post" action=(format!("/credentials/{}", credential.id)) {
                    (csrf_field(csrf_token))
                    div.grid {
                        label.field {
                            (t.field_name)
                            input name="name" value=(credential.name) required;
                        }
                        label.field {
                            (t.field_username)
                            input name="username" value=(credential.username) required;
                        }
                        label.field {
                            (t.field_auth_type)
                            select name="auth_type" {
                                option value="password" selected[credential.auth_type == "password"] { "password" }
                                option value="key" selected[credential.auth_type == "key"] { "key" }
                                option value="key+passphrase" selected[credential.auth_type == "key+passphrase"] { "key+passphrase" }
                            }
                        }
                        label.field {
                            (t.replace_password)
                            input type="password" name="password";
                        }
                        label.field {
                            (t.replace_passphrase)
                            input type="password" name="passphrase";
                        }
                        label.field.field-wide {
                            (t.replace_private_key)
                            textarea name="private_key" rows="8" {}
                        }
                    }
                    p.fine-print { (t.secret_keep_note) }
                    div.button-row {
                        button type="submit" { (t.save_changes) }
                        a.ghost-button href="/credentials" { (t.back_to_credentials) }
                    }
                }
            }
        },
    )
}

pub fn keys(t: &L10n, items: &[AuthorizedKey], csrf_token: &str) -> Markup {
    layout(
        t.keys_title,
        "keys",
        t,
        html! {
            div.page-intro {
                h2 { (t.keys_heading) }
                p { (t.keys_intro) }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.keys_add_heading) }
                        p { (t.keys_add_intro) }
                    }
                }
                form method="post" action="/keys" {
                    (csrf_field(csrf_token))
                    div.grid {
                        label.field {
                            (t.field_name)
                            input name="name" required;
                        }
                        label.field.field-wide {
                            (t.field_public_key)
                            textarea name="public_key" rows="4" required {}
                        }
                    }
                    div.button-row {
                        button type="submit" { (t.save_key) }
                    }
                }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.keys_existing_heading) }
                        p { (t.keys_existing_intro) }
                    }
                }
                div.table-wrap {
                    table.data-table {
                        thead {
                            tr { th { (t.field_name) } th { (t.field_fingerprint) } th { (t.field_status) } th { (t.field_action) } }
                        }
                        tbody {
                            @if items.is_empty() {
                                tr.empty-row { td colspan="4" { (t.no_keys) } }
                            }
                            @for key in items {
                                tr {
                                    td {
                                        div.primary-cell {
                                            (key.name)
                                            @if let Some(created_at) = &key.created_at {
                                                span.subtle { (t.key_added_prefix) " " (created_at) }
                                            }
                                        }
                                    }
                                    td.mono { (key.fingerprint) }
                                    td {
                                        @if key.is_active {
                                            span.status-pill { (t.active) }
                                        } @else {
                                            span.status-pill.neutral { (t.inactive) }
                                        }
                                    }
                                    td {
                                        div.action-row {
                                            a class="button" href=(format!("/keys/{}/edit", key.id)) { (t.edit) }
                                            @if key.is_active {
                                                form method="post" action=(format!("/keys/{}/deactivate", key.id)) {
                                                    (csrf_field(csrf_token))
                                                    button class="danger" type="submit" { (t.deactivate) }
                                                }
                                            } @else {
                                                form method="post" action=(format!("/keys/{}/activate", key.id)) {
                                                    (csrf_field(csrf_token))
                                                    button type="submit" { (t.activate) }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
    )
}

pub fn edit_key(t: &L10n, key: &AuthorizedKey, csrf_token: &str) -> Markup {
    layout(
        t.edit_key_title,
        "keys",
        t,
        html! {
            div.page-intro {
                h2 { (key.name) }
                p { (t.edit_key_intro) }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.key_details_heading) }
                        p { (t.key_details_intro) }
                    }
                }
                form method="post" action=(format!("/keys/{}", key.id)) {
                    (csrf_field(csrf_token))
                    div.grid {
                        label.field {
                            (t.field_name)
                            input name="name" value=(key.name) required;
                        }
                        label.field.field-wide {
                            (t.field_public_key)
                            textarea name="public_key" rows="4" required { (key.public_key) }
                        }
                    }
                    div.button-row {
                        button type="submit" { (t.save_changes) }
                        a.ghost-button href="/keys" { (t.back_to_keys) }
                    }
                }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.danger_zone) }
                        p { (t.delete_key_intro) }
                    }
                }
                form method="post" action=(format!("/keys/{}/delete", key.id)) {
                    (csrf_field(csrf_token))
                    button class="danger" type="submit" { (t.delete_key) }
                }
            }
        },
    )
}

pub fn known_hosts(t: &L10n, items: &[KnownHost], csrf_token: &str) -> Markup {
    layout(
        t.known_hosts_title,
        "known-hosts",
        t,
        html! {
            div.page-intro {
                h2 { (t.known_hosts_heading) }
                p { (t.known_hosts_intro) }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.known_hosts_panel_heading) }
                        p { (t.known_hosts_panel_intro) }
                    }
                }
                div.table-wrap {
                    table.data-table {
                        thead {
                            tr { th { (t.host_column) } th { (t.key_type_column) } th { (t.field_fingerprint) } th { (t.first_seen_column) } th { (t.field_action) } }
                        }
                        tbody {
                            @if items.is_empty() {
                                tr.empty-row { td colspan="5" { (t.no_known_hosts) } }
                            }
                            @for host in items {
                                tr {
                                    td {
                                        div.primary-cell {
                                            (host.hostname)
                                            span.subtle.mono { ":" (host.port) }
                                        }
                                    }
                                    td { span.status-pill.neutral { (host.key_type) } }
                                    td.mono { (host.fingerprint) }
                                    td { (host.first_seen.as_deref().unwrap_or("-")) }
                                    td {
                                        div.action-row {
                                            form method="post" action=(format!("/known-hosts/{}/{}/delete", host.hostname, host.port)) {
                                                (csrf_field(csrf_token))
                                                input type="hidden" name="key_type" value=(host.key_type);
                                                button class="danger" type="submit" { (t.delete) }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
    )
}

pub fn sessions(t: &L10n, items: &[Session]) -> Markup {
    layout(
        t.sessions_title,
        "sessions",
        t,
        html! {
            div.page-intro {
                h2 { (t.sessions_heading) }
                p { (t.sessions_intro) }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.sessions_recent_heading) }
                        p { (t.sessions_recent_intro) }
                    }
                }
                div.table-wrap {
                    table.data-table {
                        thead {
                            tr { th { (t.started_column) } th { (t.mode_column) } th { (t.key_column) } th { (t.asset_column) } th { (t.target_column) } th { (t.field_status) } th { (t.error_column) } }
                        }
                        tbody {
                            @if items.is_empty() {
                                tr.empty-row { td colspan="7" { (t.no_sessions) } }
                            }
                            @for session in items {
                                tr {
                                    td { (session.started_at.as_deref().unwrap_or("-")) }
                                    td { span.status-pill.neutral { (session.mode) } }
                                    td {
                                        div.primary-cell {
                                            (session.key_name.as_deref().unwrap_or("-"))
                                            span.subtle.mono { (session.key_finger) }
                                        }
                                    }
                                    td { (session.asset_name.as_deref().unwrap_or("-")) }
                                    td.mono {
                                        @if let Some(target_host) = &session.target_host {
                                            (target_host) ":" (session.target_port.unwrap_or_default())
                                        } @else {
                                            "-"
                                        }
                                    }
                                    td {
                                        @if session.status == "failed" {
                                            span.status-pill.danger { (session.status) }
                                        } @else if session.status == "ok" {
                                            span.status-pill { (session.status) }
                                        } @else {
                                            span.status-pill.neutral { (session.status) }
                                        }
                                    }
                                    td { (session.error.as_deref().unwrap_or("")) }
                                }
                            }
                        }
                    }
                }
            }
        },
    )
}

pub fn import_export(t: &L10n, csrf_token: &str, summary: Option<&ImportSummary>) -> Markup {
    layout(
        t.import_title,
        "import",
        t,
        html! {
            div.page-intro {
                h2 { (t.import_heading) }
                p { (t.import_intro) }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.import_form_heading) }
                        p { (t.import_form_intro) }
                    }
                }
                form method="post" action="/import" enctype="multipart/form-data" {
                    (csrf_field(csrf_token))
                    div.grid {
                        label.field {
                            (t.import_kind)
                            select name="kind" {
                                option value="assets" { (t.kind_assets) }
                                option value="credentials" { (t.kind_credentials) }
                            }
                        }
                        label.field {
                            (t.import_format)
                            select name="format" {
                                option value="csv" { "CSV" }
                                option value="json" { "JSON" }
                            }
                        }
                        label.field {
                            (t.import_conflict)
                            select name="on_conflict" {
                                option value="skip" { (t.conflict_skip) }
                                option value="overwrite" { (t.conflict_overwrite) }
                                option value="error" { (t.conflict_error) }
                            }
                        }
                        label.field.field-wide {
                            (t.import_file)
                            input type="file" name="file" required;
                        }
                    }
                    div.button-row {
                        button type="submit" { (t.import_submit) }
                    }
                }
            }
            @if let Some(summary) = summary {
                section.panel {
                    div.panel-header {
                        div {
                            h2 { (t.import_summary) }
                        }
                    }
                    div.import-summary {
                        p { (t.imported) ": " (summary.imported) }
                        p { (t.skipped) ": " (summary.skipped) }
                        p { (t.overwritten) ": " (summary.overwritten) }
                        @if !summary.errors.is_empty() {
                            p { (t.errors) ":" }
                            ul {
                                @for error in &summary.errors {
                                    li { (error) }
                                }
                            }
                        }
                    }
                }
            }
        },
    )
}

fn nav_link(href: &str, label: &str, active: bool) -> Markup {
    if active {
        html! {
            a class="nav-link active" href=(href) aria-current="page" { (label) }
        }
    } else {
        html! {
            a class="nav-link" href=(href) { (label) }
        }
    }
}

fn csrf_field(csrf_token: &str) -> Markup {
    html! {
        input type="hidden" name="csrf_token" value=(csrf_token);
    }
}

fn language_switch_href(locale: Locale, active: &str) -> String {
    format!(
        "/set-language?lang={}&redirect={}",
        locale.cookie_value(),
        url_query_value(active_path(active))
    )
}

fn active_path(active: &str) -> &'static str {
    match active {
        "overview" => "/",
        "assets" => "/assets",
        "credentials" => "/credentials",
        "keys" => "/keys",
        "known-hosts" => "/known-hosts",
        "sessions" => "/sessions",
        "import" => "/import",
        "login" => "/login",
        _ => "/",
    }
}

fn url_query_value(value: &str) -> String {
    let mut output = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                output.push(byte as char);
            }
            _ => {
                use std::fmt::Write as _;
                let _ = write!(&mut output, "%{byte:02X}");
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::super::i18n::EN;
    use super::*;

    #[test]
    fn mutating_forms_include_csrf_token() {
        let rendered = assets(&EN, &[], &[], "csrf-123", None, &[]).into_string();

        assert!(rendered.contains(r#"name="csrf_token""#));
        assert!(rendered.contains(r#"value="csrf-123""#));
    }

    #[test]
    fn layout_renders_admin_shell_and_active_navigation() {
        let rendered =
            layout(EN.assets_title, "assets", &EN, html! { p { "content" } }).into_string();

        assert!(rendered.contains(r#"class="admin-shell""#));
        assert!(rendered.contains(r#"aria-current="page""#));
        assert!(rendered.contains(r#"href="/assets""#));
        assert!(rendered.contains("/set-language?lang=zh"));
    }

    #[test]
    fn overview_renders_metric_tiles_with_labels() {
        let rendered = overview(&EN, 2, 3, 4, 5).into_string();

        assert!(rendered.contains(r#"class="metric-grid""#));
        assert!(rendered.contains(r#"class="metric-value""#));
        assert!(rendered.contains("Managed assets"));
        assert!(rendered.contains("Recent sessions"));
    }

    #[test]
    fn assets_page_renders_tag_filters_and_bulk_editor() {
        let tags = vec!["prod".to_string(), "web".to_string()];
        let rendered = assets(&EN, &[], &[], "csrf-123", Some("prod"), &tags).into_string();

        assert!(rendered.contains(r#"href="/assets?tag=prod""#));
        assert!(rendered.contains(r#"action="/assets/bulk-tags""#));
        assert!(rendered.contains(r#"list="asset-tags-list""#));
    }

    #[test]
    fn import_page_uses_multipart_upload_form() {
        let rendered = import_export(&EN, "csrf-123", None).into_string();

        assert!(rendered.contains(r#"enctype="multipart/form-data""#));
        assert!(rendered.contains(r#"type="file""#));
    }
}
