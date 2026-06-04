use hop_core::{Asset, AuthorizedKey, Credential, KnownHost, Session};
use maud::{html, Markup, DOCTYPE};

pub fn layout(title: &str, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) " - Hop Admin" }
                style {
                    r#"
                    body { font-family: system-ui, sans-serif; margin: 0; background: #f7f7f5; color: #222; }
                    header { background: #202124; color: white; padding: 12px 24px; display: flex; gap: 16px; align-items: center; }
                    header a { color: white; text-decoration: none; }
                    main { max-width: 1120px; margin: 24px auto; padding: 0 20px; }
                    table { width: 100%; border-collapse: collapse; background: white; }
                    th, td { border-bottom: 1px solid #ddd; padding: 8px; text-align: left; vertical-align: top; }
                    input, select, textarea { width: 100%; box-sizing: border-box; padding: 7px; margin: 4px 0 12px; }
                    button, .button { background: #1f6feb; color: white; border: 0; padding: 8px 12px; text-decoration: none; display: inline-block; cursor: pointer; }
                    .danger { background: #b42318; }
                    .muted { color: #666; }
                    .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap: 12px; }
                    pre { white-space: pre-wrap; background: white; padding: 12px; }
                    "#
                }
            }
            body {
                header {
                    strong { "Hop Admin" }
                    a href="/" { "Overview" }
                    a href="/assets" { "Assets" }
                    a href="/credentials" { "Credentials" }
                    a href="/keys" { "Keys" }
                    a href="/known-hosts" { "Known Hosts" }
                    a href="/sessions" { "Sessions" }
                    a href="/logout" { "Logout" }
                }
                main { (body) }
            }
        }
    }
}

pub fn login(error: Option<&str>) -> Markup {
    layout(
        "Login",
        html! {
            h1 { "Admin Login" }
            @if let Some(error) = error {
                p style="color:#b42318" { (error) }
            }
            form method="post" action="/login" {
                label { "Password" input type="password" name="password" required; }
                button type="submit" { "Login" }
            }
        },
    )
}

pub fn overview(asset_count: usize, credential_count: usize, key_count: usize, session_count: usize) -> Markup {
    layout(
        "Overview",
        html! {
            h1 { "Overview" }
            div.grid {
                p { strong { (asset_count) } br; "assets" }
                p { strong { (credential_count) } br; "credentials" }
                p { strong { (key_count) } br; "authorized keys" }
                p { strong { (session_count) } br; "recent sessions" }
            }
        },
    )
}

pub fn assets(items: &[Asset], credentials: &[Credential]) -> Markup {
    layout(
        "Assets",
        html! {
            h1 { "Assets" }
            h2 { "Add Asset" }
            form method="post" action="/assets" {
                div.grid {
                    label { "Name" input name="name" required; }
                    label { "Hostname" input name="hostname" required; }
                    label { "Port" input name="port" type="number" value="22" required; }
                    label { "Tags (comma separated)" input name="tags"; }
                    label { "Credential"
                        select name="credential_id" {
                            option value="" { "Proxy only / no managed credential" }
                            @for credential in credentials {
                                option value=(credential.id) { (credential.name) " (" (credential.username) ")" }
                            }
                        }
                    }
                }
                label { "Description" textarea name="description" {} }
                button type="submit" { "Save" }
            }
            h2 { "Existing Assets" }
            table {
                tr { th { "Name" } th { "Target" } th { "Tags" } th { "Credential" } th { "Action" } }
                @for asset in items {
                    tr {
                        td { (asset.name) }
                        td { (asset.hostname) ":" (asset.port) }
                        td { (asset.tags.join(", ")) }
                        td { (asset.credential_id.as_deref().unwrap_or("-")) }
                        td {
                            a class="button" href=(format!("/assets/{}/edit", asset.id)) { "Edit" }
                            " "
                            form method="post" action=(format!("/assets/{}/delete", asset.id)) {
                                button class="danger" type="submit" { "Delete" }
                            }
                        }
                    }
                }
            }
        },
    )
}

pub fn edit_asset(asset: &Asset, credentials: &[Credential]) -> Markup {
    layout(
        "Edit Asset",
        html! {
            h1 { "Edit Asset" }
            form method="post" action=(format!("/assets/{}", asset.id)) {
                div.grid {
                    label { "Name" input name="name" value=(asset.name) required; }
                    label { "Hostname" input name="hostname" value=(asset.hostname) required; }
                    label { "Port" input name="port" type="number" value=(asset.port) required; }
                    label { "Tags (comma separated)" input name="tags" value=(asset.tags.join(",")); }
                    label { "Credential"
                        select name="credential_id" {
                            option value="" selected[asset.credential_id.is_none()] { "Proxy only / no managed credential" }
                            @for credential in credentials {
                                option value=(credential.id) selected[asset.credential_id.as_deref() == Some(credential.id.as_str())] {
                                    (credential.name) " (" (credential.username) ")"
                                }
                            }
                        }
                    }
                }
                label { "Description" textarea name="description" { (asset.description.as_deref().unwrap_or("")) } }
                button type="submit" { "Save" }
            }
        },
    )
}

pub fn credentials(items: &[Credential]) -> Markup {
    layout(
        "Credentials",
        html! {
            h1 { "Credentials" }
            h2 { "Add Credential" }
            form method="post" action="/credentials" {
                div.grid {
                    label { "Name" input name="name" required; }
                    label { "Username" input name="username" required; }
                    label { "Auth Type"
                        select name="auth_type" {
                            option value="password" { "password" }
                            option value="key" { "key" }
                            option value="key+passphrase" { "key+passphrase" }
                        }
                    }
                }
                label { "Password" input type="password" name="password"; }
                label { "Private Key" textarea name="private_key" rows="8" {} }
                label { "Passphrase" input type="password" name="passphrase"; }
                button type="submit" { "Save" }
                p.muted { "Secrets are encrypted before storage and are never rendered back after save." }
            }
            h2 { "Existing Credentials" }
            table {
                tr { th { "Name" } th { "Username" } th { "Type" } th { "Secrets" } th { "Action" } }
                @for credential in items {
                    tr {
                        td { (credential.name) }
                        td { (credential.username) }
                        td { (credential.auth_type) }
                        td {
                            @if credential.password_enc.is_some() { "password " }
                            @if credential.private_key_enc.is_some() { "private_key " }
                            @if credential.passphrase_enc.is_some() { "passphrase" }
                        }
                        td {
                            a class="button" href=(format!("/credentials/{}/edit", credential.id)) { "Edit" }
                            " "
                            form method="post" action=(format!("/credentials/{}/delete", credential.id)) {
                                button class="danger" type="submit" { "Delete" }
                            }
                        }
                    }
                }
            }
        },
    )
}

pub fn edit_credential(credential: &Credential) -> Markup {
    layout(
        "Edit Credential",
        html! {
            h1 { "Edit Credential" }
            form method="post" action=(format!("/credentials/{}", credential.id)) {
                div.grid {
                    label { "Name" input name="name" value=(credential.name) required; }
                    label { "Username" input name="username" value=(credential.username) required; }
                    label { "Auth Type"
                        select name="auth_type" {
                            option value="password" selected[credential.auth_type == "password"] { "password" }
                            option value="key" selected[credential.auth_type == "key"] { "key" }
                            option value="key+passphrase" selected[credential.auth_type == "key+passphrase"] { "key+passphrase" }
                        }
                    }
                }
                label { "Replace Password" input type="password" name="password"; }
                label { "Replace Private Key" textarea name="private_key" rows="8" {} }
                label { "Replace Passphrase" input type="password" name="passphrase"; }
                p.muted { "Leave secret fields blank to keep existing encrypted values. Existing secrets are not rendered back." }
                button type="submit" { "Save" }
            }
        },
    )
}

pub fn keys(items: &[AuthorizedKey]) -> Markup {
    layout(
        "Keys",
        html! {
            h1 { "Authorized Keys" }
            h2 { "Add Key" }
            form method="post" action="/keys" {
                label { "Name" input name="name" required; }
                label { "Public Key" textarea name="public_key" rows="4" required {} }
                button type="submit" { "Save" }
            }
            h2 { "Existing Keys" }
            table {
                tr { th { "Name" } th { "Fingerprint" } th { "Status" } th { "Action" } }
                @for key in items {
                    tr {
                        td { (key.name) }
                        td { (key.fingerprint) }
                        td { @if key.is_active { "active" } @else { "inactive" } }
                        td {
                            a class="button" href=(format!("/keys/{}/edit", key.id)) { "Edit" }
                            " "
                            @if key.is_active {
                                form method="post" action=(format!("/keys/{}/deactivate", key.id)) {
                                    button class="danger" type="submit" { "Deactivate" }
                                }
                            } @else {
                                form method="post" action=(format!("/keys/{}/activate", key.id)) {
                                    button type="submit" { "Activate" }
                                }
                            }
                        }
                    }
                }
            }
        },
    )
}

pub fn edit_key(key: &AuthorizedKey) -> Markup {
    layout(
        "Edit Key",
        html! {
            h1 { "Edit Key" }
            form method="post" action=(format!("/keys/{}", key.id)) {
                label { "Name" input name="name" value=(key.name) required; }
                label { "Public Key" textarea name="public_key" rows="4" required { (key.public_key) } }
                button type="submit" { "Save" }
            }
            form method="post" action=(format!("/keys/{}/delete", key.id)) {
                button class="danger" type="submit" { "Delete Key" }
            }
        },
    )
}

pub fn known_hosts(items: &[KnownHost]) -> Markup {
    layout(
        "Known Hosts",
        html! {
            h1 { "Known Hosts" }
            table {
                tr { th { "Host" } th { "Key Type" } th { "Fingerprint" } th { "Action" } }
                @for host in items {
                    tr {
                        td { (host.hostname) ":" (host.port) }
                        td { (host.key_type) }
                        td { (host.fingerprint) }
                        td {
                            form method="post" action=(format!("/known-hosts/{}/{}/delete", host.hostname, host.port)) {
                                input type="hidden" name="key_type" value=(host.key_type);
                                button class="danger" type="submit" { "Delete" }
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
            h1 { "Sessions" }
            table {
                tr { th { "Started" } th { "Mode" } th { "Key" } th { "Asset" } th { "Target" } th { "Status" } th { "Error" } }
                @for session in items {
                    tr {
                        td { (session.started_at.as_deref().unwrap_or("-")) }
                        td { (session.mode) }
                        td { (session.key_name.as_deref().unwrap_or("-")) br; small { (session.key_finger) } }
                        td { (session.asset_name.as_deref().unwrap_or("-")) }
                        td { (session.target_host.as_deref().unwrap_or("-")) ":" (session.target_port.unwrap_or_default()) }
                        td { (session.status) }
                        td { (session.error.as_deref().unwrap_or("")) }
                    }
                }
            }
        },
    )
}
