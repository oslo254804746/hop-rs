use hop_core::{Asset, AuthorizedKey, Credential, KnownHost, Session};
use maud::{html, Markup, DOCTYPE};

pub fn layout(title: &str, body_content: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) " - Hop Admin" }
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
                                span { "Admin Console" }
                            }
                        }
                        nav.nav aria-label="Primary" {
                            (nav_link("/", "Overview", title == "Overview"))
                            (nav_link("/assets", "Assets", title == "Assets" || title == "Edit Asset"))
                            (nav_link("/credentials", "Credentials", title == "Credentials" || title == "Edit Credential"))
                            (nav_link("/keys", "Keys", title == "Keys" || title == "Authorized Keys" || title == "Edit Key"))
                            (nav_link("/known-hosts", "Known Hosts", title == "Known Hosts"))
                            (nav_link("/sessions", "Sessions", title == "Sessions"))
                        }
                        div.sidebar-footer {
                            span.status-dot {}
                            "Loopback admin"
                            small { "Keep this surface behind a local tunnel or trusted management network." }
                        }
                    }
                    div.content-shell {
                        header.topbar {
                            div {
                                p.eyebrow { "Hop Admin Web" }
                                h1 { (title) }
                            }
                            @if title != "Login" {
                                a.ghost-button href="/logout" { "Logout" }
                            }
                        }
                        main.workspace { (body_content) }
                    }
                }
            }
        }
    }
}

pub fn login(error: Option<&str>) -> Markup {
    layout(
        "Login",
        html! {
            div.login-wrap {
                section.panel {
                    div.panel-header {
                        div {
                            h2 { "Admin Login" }
                            p { "Sign in to manage assets, trusted keys, credentials, and recent SSH activity." }
                        }
                    }
                    @if let Some(error) = error {
                        p.error-message { (error) }
                    }
                    form method="post" action="/login" {
                        label.field {
                            "Password"
                            input type="password" name="password" required;
                        }
                        div.button-row {
                            button type="submit" { "Login" }
                        }
                    }
                }
            }
        },
    )
}

pub fn overview(
    asset_count: usize,
    credential_count: usize,
    key_count: usize,
    session_count: usize,
) -> Markup {
    layout(
        "Overview",
        html! {
            div.page-intro {
                h2 { "Operational snapshot" }
                p { "A compact view of the surfaces Hop currently protects and the administrative data available to the SSH service." }
            }
            div.metric-grid {
                div.metric {
                    span.metric-label { "Managed assets" }
                    strong.metric-value { (asset_count) }
                    span.metric-note { "Targets available to TUI and allowlist checks" }
                }
                div.metric {
                    span.metric-label { "Stored credentials" }
                    strong.metric-value { (credential_count) }
                    span.metric-note { "Encrypted target connection material" }
                }
                div.metric {
                    span.metric-label { "Authorized keys" }
                    strong.metric-value { (key_count) }
                    span.metric-note { "SSH identities allowed into Hop" }
                }
                div.metric {
                    span.metric-label { "Recent sessions" }
                    strong.metric-value { (session_count) }
                    span.metric-note { "Latest activity retained for review" }
                }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { "Admin scope" }
                        p { "This web surface is intentionally focused on inventory, credential custody, trust records, and session review." }
                    }
                }
                p.fine-print { "Keep the Admin Web listener bound to loopback and use an SSH tunnel for remote administration." }
            }
        },
    )
}

pub fn assets(items: &[Asset], credentials: &[Credential], csrf_token: &str) -> Markup {
    layout(
        "Assets",
        html! {
            div.page-intro {
                h2 { "Asset inventory" }
                p { "Define SSH targets that users can find in the TUI or reach through the ProxyJump allowlist." }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { "Add asset" }
                        p { "Create a named target with optional tags and a managed credential." }
                    }
                }
                form method="post" action="/assets" {
                    (csrf_field(csrf_token))
                    div.grid {
                        label.field {
                            "Name"
                            input name="name" required;
                        }
                        label.field {
                            "Hostname"
                            input name="hostname" required;
                        }
                        label.field {
                            "Port"
                            input name="port" type="number" value="22" required;
                        }
                        label.field {
                            "Tags"
                            input name="tags" placeholder="prod, web";
                        }
                        label.field {
                            "Credential"
                            select name="credential_id" {
                                option value="" { "Proxy only / no managed credential" }
                                @for credential in credentials {
                                    option value=(credential.id) { (credential.name) " (" (credential.username) ")" }
                                }
                            }
                        }
                        label.field.field-wide {
                            "Description"
                            textarea name="description" {}
                        }
                    }
                    div.button-row {
                        button type="submit" { "Save asset" }
                    }
                }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { "Existing assets" }
                        p { "Review address, routing tags, and whether Hop can use a stored credential for managed connections." }
                    }
                }
                div.table-wrap {
                    table.data-table {
                        thead {
                            tr { th { "Name" } th { "Target" } th { "Tags" } th { "Credential" } th { "Action" } }
                        }
                        tbody {
                            @if items.is_empty() {
                                tr.empty-row { td colspan="5" { "No assets have been added yet." } }
                            }
                            @for asset in items {
                                tr {
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
                                                span.status-pill.neutral { "untagged" }
                                            }
                                            @for tag in &asset.tags {
                                                span.tag { (tag) }
                                            }
                                        }
                                    }
                                    td {
                                        @if let Some(credential_id) = &asset.credential_id {
                                            span.status-pill { (credential_id) }
                                        } @else {
                                            span.status-pill.neutral { "proxy only" }
                                        }
                                    }
                                    td {
                                        div.action-row {
                                            a class="button" href=(format!("/assets/{}/edit", asset.id)) { "Edit" }
                                            form method="post" action=(format!("/assets/{}/delete", asset.id)) {
                                                (csrf_field(csrf_token))
                                                button class="danger" type="submit" { "Delete" }
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

pub fn edit_asset(asset: &Asset, credentials: &[Credential], csrf_token: &str) -> Markup {
    layout(
        "Edit Asset",
        html! {
            div.page-intro {
                h2 { (asset.name) }
                p { "Update the target address, tags, description, or managed credential attached to this asset." }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { "Asset details" }
                        p { "Changes are used by both managed TUI connections and allowlist matching." }
                    }
                }
                form method="post" action=(format!("/assets/{}", asset.id)) {
                    (csrf_field(csrf_token))
                    div.grid {
                        label.field {
                            "Name"
                            input name="name" value=(asset.name) required;
                        }
                        label.field {
                            "Hostname"
                            input name="hostname" value=(asset.hostname) required;
                        }
                        label.field {
                            "Port"
                            input name="port" type="number" value=(asset.port) required;
                        }
                        label.field {
                            "Tags"
                            input name="tags" value=(asset.tags.join(",")) placeholder="prod, web";
                        }
                        label.field {
                            "Credential"
                            select name="credential_id" {
                                option value="" selected[asset.credential_id.is_none()] { "Proxy only / no managed credential" }
                                @for credential in credentials {
                                    option value=(credential.id) selected[asset.credential_id.as_deref() == Some(credential.id.as_str())] {
                                        (credential.name) " (" (credential.username) ")"
                                    }
                                }
                            }
                        }
                        label.field.field-wide {
                            "Description"
                            textarea name="description" { (asset.description.as_deref().unwrap_or("")) }
                        }
                    }
                    div.button-row {
                        button type="submit" { "Save changes" }
                        a.ghost-button href="/assets" { "Back to assets" }
                    }
                }
            }
        },
    )
}

pub fn credentials(items: &[Credential], csrf_token: &str) -> Markup {
    layout(
        "Credentials",
        html! {
            div.page-intro {
                h2 { "Credential custody" }
                p { "Store target-side SSH material for server-managed connections. Secrets are encrypted before they reach SQLite." }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { "Add credential" }
                        p { "Choose the target username and authentication mode, then provide the matching secret material." }
                    }
                }
                form method="post" action="/credentials" {
                    (csrf_field(csrf_token))
                    div.grid {
                        label.field {
                            "Name"
                            input name="name" required;
                        }
                        label.field {
                            "Username"
                            input name="username" required;
                        }
                        label.field {
                            "Auth type"
                            select name="auth_type" {
                                option value="password" { "password" }
                                option value="key" { "key" }
                                option value="key+passphrase" { "key+passphrase" }
                            }
                        }
                        label.field {
                            "Password"
                            input type="password" name="password";
                        }
                        label.field {
                            "Passphrase"
                            input type="password" name="passphrase";
                        }
                        label.field.field-wide {
                            "Private key"
                            textarea name="private_key" rows="8" {}
                        }
                    }
                    div.button-row {
                        button type="submit" { "Save credential" }
                    }
                    p.fine-print { "Secrets are encrypted before storage and are never rendered back after save." }
                }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { "Existing credentials" }
                        p { "Audit target users, auth modes, and which encrypted secret fields are present." }
                    }
                }
                div.table-wrap {
                    table.data-table {
                        thead {
                            tr { th { "Name" } th { "Username" } th { "Type" } th { "Secrets" } th { "Action" } }
                        }
                        tbody {
                            @if items.is_empty() {
                                tr.empty-row { td colspan="5" { "No credentials have been stored yet." } }
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
                                                span.status-pill.neutral { "none" }
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
                                            a class="button" href=(format!("/credentials/{}/edit", credential.id)) { "Edit" }
                                            form method="post" action=(format!("/credentials/{}/delete", credential.id)) {
                                                (csrf_field(csrf_token))
                                                button class="danger" type="submit" { "Delete" }
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

pub fn edit_credential(credential: &Credential, csrf_token: &str) -> Markup {
    layout(
        "Edit Credential",
        html! {
            div.page-intro {
                h2 { (credential.name) }
                p { "Edit identity metadata or replace encrypted secret fields. Blank secret fields keep their existing values." }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { "Credential details" }
                        p { "Existing secrets are not rendered back; enter a replacement only when rotating material." }
                    }
                }
                form method="post" action=(format!("/credentials/{}", credential.id)) {
                    (csrf_field(csrf_token))
                    div.grid {
                        label.field {
                            "Name"
                            input name="name" value=(credential.name) required;
                        }
                        label.field {
                            "Username"
                            input name="username" value=(credential.username) required;
                        }
                        label.field {
                            "Auth type"
                            select name="auth_type" {
                                option value="password" selected[credential.auth_type == "password"] { "password" }
                                option value="key" selected[credential.auth_type == "key"] { "key" }
                                option value="key+passphrase" selected[credential.auth_type == "key+passphrase"] { "key+passphrase" }
                            }
                        }
                        label.field {
                            "Replace password"
                            input type="password" name="password";
                        }
                        label.field {
                            "Replace passphrase"
                            input type="password" name="passphrase";
                        }
                        label.field.field-wide {
                            "Replace private key"
                            textarea name="private_key" rows="8" {}
                        }
                    }
                    p.fine-print { "Leave secret fields blank to keep existing encrypted values." }
                    div.button-row {
                        button type="submit" { "Save changes" }
                        a.ghost-button href="/credentials" { "Back to credentials" }
                    }
                }
            }
        },
    )
}

pub fn keys(items: &[AuthorizedKey], csrf_token: &str) -> Markup {
    layout(
        "Keys",
        html! {
            div.page-intro {
                h2 { "SSH entry allowlist" }
                p { "Only active public keys in this list can authenticate to the Hop SSH service." }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { "Add key" }
                        p { "Paste an OpenSSH public key and give it a human-readable owner label." }
                    }
                }
                form method="post" action="/keys" {
                    (csrf_field(csrf_token))
                    div.grid {
                        label.field {
                            "Name"
                            input name="name" required;
                        }
                        label.field.field-wide {
                            "Public key"
                            textarea name="public_key" rows="4" required {}
                        }
                    }
                    div.button-row {
                        button type="submit" { "Save key" }
                    }
                }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { "Existing keys" }
                        p { "Review fingerprints and quickly suspend access without deleting the record." }
                    }
                }
                div.table-wrap {
                    table.data-table {
                        thead {
                            tr { th { "Name" } th { "Fingerprint" } th { "Status" } th { "Action" } }
                        }
                        tbody {
                            @if items.is_empty() {
                                tr.empty-row { td colspan="4" { "No SSH keys have been authorized yet." } }
                            }
                            @for key in items {
                                tr {
                                    td {
                                        div.primary-cell {
                                            (key.name)
                                            @if let Some(created_at) = &key.created_at {
                                                span.subtle { "Added " (created_at) }
                                            }
                                        }
                                    }
                                    td.mono { (key.fingerprint) }
                                    td {
                                        @if key.is_active {
                                            span.status-pill { "active" }
                                        } @else {
                                            span.status-pill.neutral { "inactive" }
                                        }
                                    }
                                    td {
                                        div.action-row {
                                            a class="button" href=(format!("/keys/{}/edit", key.id)) { "Edit" }
                                            @if key.is_active {
                                                form method="post" action=(format!("/keys/{}/deactivate", key.id)) {
                                                    (csrf_field(csrf_token))
                                                    button class="danger" type="submit" { "Deactivate" }
                                                }
                                            } @else {
                                                form method="post" action=(format!("/keys/{}/activate", key.id)) {
                                                    (csrf_field(csrf_token))
                                                    button type="submit" { "Activate" }
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

pub fn edit_key(key: &AuthorizedKey, csrf_token: &str) -> Markup {
    layout(
        "Edit Key",
        html! {
            div.page-intro {
                h2 { (key.name) }
                p { "Update the owner label or replace the public key material associated with this allowlist entry." }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { "Key details" }
                        p { "Fingerprint changes after replacing the public key are reflected when the record is saved." }
                    }
                }
                form method="post" action=(format!("/keys/{}", key.id)) {
                    (csrf_field(csrf_token))
                    div.grid {
                        label.field {
                            "Name"
                            input name="name" value=(key.name) required;
                        }
                        label.field.field-wide {
                            "Public key"
                            textarea name="public_key" rows="4" required { (key.public_key) }
                        }
                    }
                    div.button-row {
                        button type="submit" { "Save changes" }
                        a.ghost-button href="/keys" { "Back to keys" }
                    }
                }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { "Danger zone" }
                        p { "Deleting this entry immediately removes the key from the SSH allowlist." }
                    }
                }
                form method="post" action=(format!("/keys/{}/delete", key.id)) {
                    (csrf_field(csrf_token))
                    button class="danger" type="submit" { "Delete key" }
                }
            }
        },
    )
}

pub fn known_hosts(items: &[KnownHost], csrf_token: &str) -> Markup {
    layout(
        "Known Hosts",
        html! {
            div.page-intro {
                h2 { "TOFU trust records" }
                p { "Review target host keys accepted by Trust On First Use. Delete a record when a host has been rebuilt or rotated." }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { "Known hosts" }
                        p { "Each fingerprint is matched on future managed connections from Hop to the target." }
                    }
                }
                div.table-wrap {
                    table.data-table {
                        thead {
                            tr { th { "Host" } th { "Key type" } th { "Fingerprint" } th { "First seen" } th { "Action" } }
                        }
                        tbody {
                            @if items.is_empty() {
                                tr.empty-row { td colspan="5" { "No target host keys have been trusted yet." } }
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
                                                button class="danger" type="submit" { "Delete" }
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

pub fn sessions(items: &[Session]) -> Markup {
    layout(
        "Sessions",
        html! {
            div.page-intro {
                h2 { "Connection activity" }
                p { "Inspect recent TUI, managed exec, and ProxyJump events recorded by the SSH service." }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { "Recent sessions" }
                        p { "Status and errors are shown without exposing interactive session contents." }
                    }
                }
                div.table-wrap {
                    table.data-table {
                        thead {
                            tr { th { "Started" } th { "Mode" } th { "Key" } th { "Asset" } th { "Target" } th { "Status" } th { "Error" } }
                        }
                        tbody {
                            @if items.is_empty() {
                                tr.empty-row { td colspan="7" { "No sessions have been recorded yet." } }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutating_forms_include_csrf_token() {
        let rendered = assets(&[], &[], "csrf-123").into_string();

        assert!(rendered.contains(r#"name="csrf_token""#));
        assert!(rendered.contains(r#"value="csrf-123""#));
    }

    #[test]
    fn layout_renders_admin_shell_and_active_navigation() {
        let rendered = layout("Assets", html! { p { "content" } }).into_string();

        assert!(rendered.contains(r#"class="admin-shell""#));
        assert!(rendered.contains(r#"aria-current="page""#));
        assert!(rendered.contains(r#"href="/assets""#));
    }

    #[test]
    fn overview_renders_metric_tiles_with_labels() {
        let rendered = overview(2, 3, 4, 5).into_string();

        assert!(rendered.contains(r#"class="metric-grid""#));
        assert!(rendered.contains(r#"class="metric-value""#));
        assert!(rendered.contains("Managed assets"));
        assert!(rendered.contains("Recent sessions"));
    }
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
