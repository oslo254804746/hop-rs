use hop_core::{
    AdminUser, Asset, AssetAccessMode, AssetHealth, AuditEvent, AuthorizedKey, Credential,
    KnownHost, Session, ADMIN_PROFILE_OPERATOR, ADMIN_PROFILE_OWNER, ADMIN_PROFILE_VIEWER,
    ASSET_HEALTH_FAILED, ASSET_HEALTH_HEALTHY, ASSET_HEALTH_UNKNOWN, ASSET_PRESET_MYSQL,
    ASSET_PRESET_POSTGRES, ASSET_PRESET_RDP, ASSET_PRESET_REDIS, ASSET_PRESET_VNC,
    ASSET_PROTOCOL_SSH, ASSET_PROTOCOL_TCP,
};
use maud::{html, Markup, PreEscaped, DOCTYPE};

use super::{
    i18n::{L10n, Locale},
    transfer::ImportSummary,
};

const ICON_OVERVIEW: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/></svg>"#;

const ICON_ASSETS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="2" width="20" height="8" rx="2"/><rect x="2" y="14" width="20" height="8" rx="2"/><circle cx="6" cy="6" r="1"/><circle cx="6" cy="18" r="1"/></svg>"#;

const ICON_CREDENTIALS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 2l-2 2m-7.61 7.61a5.5 5.5 0 1 1-7.778 7.778 5.5 5.5 0 0 1 7.777-7.777zm0 0L15.5 7.5m0 0l3 3L22 7l-3-3m-3.5 3.5L19 4"/></svg>"#;

const ICON_KEYS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 10a4 4 0 0 0-8 0c0 3 2 5.5 4 7.5C10 15.5 12 13 12 10z"/><path d="M12 10a4 4 0 0 1 8 0c0 3-2 5.5-4 7.5C14 15.5 12 13 12 10z"/><path d="M12 2v2"/><path d="M12 18v4"/></svg>"#;

const ICON_KNOWN_HOSTS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/><path d="m9 12 2 2 4-4"/></svg>"#;

const ICON_SESSIONS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/></svg>"#;

const ICON_IMPORT: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M16 3h5v5"/><path d="M8 21H3v-5"/><path d="M21 3l-9 9"/><path d="M3 21l9-9"/></svg>"#;

const ICON_SETTINGS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06A1.65 1.65 0 0 0 15 19.4a1.65 1.65 0 0 0-1 .6 1.65 1.65 0 0 0-.33 1.82V22a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 8.6 20a1.65 1.65 0 0 0-1.82-.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.6 15a1.65 1.65 0 0 0-.6-1 1.65 1.65 0 0 0-1.82-.33H2a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4 8.6a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 8.6 4a1.65 1.65 0 0 0 1-.6 1.65 1.65 0 0 0 .33-1.82V2a2 2 0 1 1 4 0v.09A1.65 1.65 0 0 0 15 4.6a1.65 1.65 0 0 0 1.82.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 8.6a1.65 1.65 0 0 0 .6 1 1.65 1.65 0 0 0 1.82.33H22a2 2 0 1 1 0 4h-.09A1.65 1.65 0 0 0 19.4 15z"/></svg>"#;

pub fn layout(title: &str, active: &str, t: &L10n, body_content: Markup) -> Markup {
    if active == "login" {
        return login_layout(title, t, body_content);
    }

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
                        color-scheme: dark;
                        --canvas: #0d1117;
                        --panel: #111827;
                        --panel-muted: #0f1724;
                        --field: #0a0f16;
                        --ink: #f9fafb;
                        --ink-soft: #cbd5e1;
                        --muted: #8b949e;
                        --border: #1f2937;
                        --border-strong: #263244;
                        --sidebar: #0a0f16;
                        --sidebar-panel: #0e1724;
                        --sidebar-ink: #f9fafb;
                        --sidebar-muted: #8b949e;
                        --control: #3b82f6;
                        --control-hover: #2563eb;
                        --secure: #22c55e;
                        --secure-soft: #0b2e1c;
                        --console-green: #22c55e;
                        --warn: #f59e0b;
                        --warn-soft: #312313;
                        --danger: #ef4444;
                        --danger-hover: #dc2626;
                        --danger-soft: #311216;
                        --row-hover: #101d2f;
                        --tag-bg: #122c4e;
                        --tag-ink: #bfdbfe;
                        --terminal: #060a10;
                        --shadow: 0 20px 54px rgba(0, 0, 0, 0.35);
                    }

                    * { box-sizing: border-box; }

                    body.admin-shell {
                        margin: 0;
                        min-height: 100vh;
                        background: var(--canvas);
                        color: var(--ink);
                        font-family: Inter, system-ui, sans-serif;
                        letter-spacing: 0;
                    }

                    a { color: inherit; }

                    a:focus-visible,
                    button:focus-visible,
                    input:focus-visible,
                    select:focus-visible,
                    textarea:focus-visible {
                        outline: 3px solid rgba(37, 99, 235, 0.28);
                        outline-offset: 2px;
                    }

                    .app-frame {
                        display: grid;
                        grid-template-columns: 272px minmax(0, 1fr);
                        min-height: 100vh;
                    }

                    .sidebar {
                        position: sticky;
                        top: 0;
                        height: 100svh;
                        padding: 22px 16px;
                        background: var(--sidebar);
                        border-right: 1px solid var(--border);
                        color: var(--sidebar-ink);
                        display: flex;
                        flex-direction: column;
                        gap: 24px;
                    }

                    .brand {
                        display: grid;
                        grid-template-columns: 40px minmax(0, 1fr);
                        gap: 12px;
                        align-items: center;
                        padding: 0 6px 14px;
                        border-bottom: 1px solid #263244;
                    }

                    .brand-mark {
                        width: 40px;
                        height: 40px;
                        border-radius: 8px;
                        display: grid;
                        place-items: center;
                        background: #122c4e;
                        border: 1px solid rgba(255, 255, 255, 0.16);
                        color: #bfdbfe;
                        font-weight: 850;
                        font-size: 1rem;
                        font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
                    }

                    .brand strong {
                        display: block;
                        font-size: 1.02rem;
                        color: #ffffff;
                    }

                    .brand span {
                        color: var(--sidebar-muted);
                        font-size: 0.82rem;
                    }

                    .nav {
                        display: grid;
                        gap: 3px;
                    }

                    .nav-link {
                        position: relative;
                        min-height: 40px;
                        display: flex;
                        align-items: center;
                        gap: 10px;
                        padding: 9px 10px 9px 14px;
                        border-radius: 8px;
                        color: var(--sidebar-ink);
                        text-decoration: none;
                        font-weight: 650;
                        font-size: 0.9rem;
                        transition: background 160ms ease, color 160ms ease;
                    }

                    .nav-link svg {
                        width: 18px;
                        height: 18px;
                        opacity: 0.72;
                        flex-shrink: 0;
                        transition: opacity 160ms ease, color 160ms ease;
                    }

                    .nav-link:hover {
                        background: #0e1724;
                        color: #ffffff;
                    }

                    .nav-link:hover svg { opacity: 1; }

                    .nav-link.active {
                        background: #0f2a4a;
                        color: #ffffff;
                        box-shadow: inset 3px 0 0 var(--control);
                    }

                    .nav-link.active svg { opacity: 1; color: var(--control); }

                    .sidebar-footer {
                        margin-top: auto;
                        padding: 14px;
                        border: 1px solid #2d3a4d;
                        border-radius: 8px;
                        background: var(--sidebar-panel);
                        color: var(--sidebar-ink);
                        font-size: 0.84rem;
                    }

                    .sidebar-footer small {
                        display: block;
                        margin-top: 5px;
                        color: var(--sidebar-muted);
                        line-height: 1.45;
                    }

                    .status-dot {
                        width: 8px;
                        height: 8px;
                        display: inline-block;
                        margin-right: 8px;
                        border-radius: 999px;
                        background: #34d399;
                    }

                    .language-switch {
                        display: flex;
                        align-items: center;
                        justify-content: space-between;
                        gap: 10px;
                        margin-top: 12px;
                        padding-top: 12px;
                        border-top: 1px solid #2d3a4d;
                    }

                    .language-switch a {
                        color: #bfdbfe;
                        font-weight: 750;
                        text-decoration: none;
                    }

                    .language-switch a:hover { color: #ffffff; }

                    .content-shell {
                        min-width: 0;
                        background: var(--canvas);
                    }

                    .topbar {
                        position: sticky;
                        top: 0;
                        z-index: 10;
                        display: flex;
                        align-items: flex-start;
                        justify-content: space-between;
                        gap: 20px;
                        padding: 26px 38px 18px;
                        border-bottom: 1px solid var(--border);
                        background: rgba(13, 17, 23, 0.92);
                        backdrop-filter: blur(12px);
                    }

                    .eyebrow {
                        margin: 0 0 6px;
                        color: var(--control);
                        font-size: 0.76rem;
                        font-weight: 800;
                        text-transform: uppercase;
                        letter-spacing: 0;
                    }

                    .topbar h1 {
                        margin: 0;
                        font-size: 2rem;
                        line-height: 1.1;
                    }

                    .workspace {
                        width: min(1280px, 100%);
                        padding: 26px 38px 58px;
                    }

                    .page-intro {
                        display: grid;
                        gap: 7px;
                        margin-bottom: 20px;
                    }

                    .page-intro h2 {
                        margin: 0;
                        font-size: 1.2rem;
                    }

                    .page-intro p {
                        margin: 0;
                        color: var(--ink-soft);
                        max-width: 800px;
                        line-height: 1.58;
                    }

                    .panel {
                        min-width: 0;
                        margin: 0 0 18px;
                        padding: 20px;
                        border: 1px solid var(--border);
                        border-radius: 8px;
                        background: var(--panel);
                        box-shadow: var(--shadow);
                    }

                    .panel-header {
                        display: flex;
                        align-items: flex-start;
                        justify-content: space-between;
                        gap: 18px;
                        margin-bottom: 16px;
                    }

                    .panel-header h2 {
                        margin: 0;
                        font-size: 1.02rem;
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
                        gap: 9px;
                        margin-top: 16px;
                    }

                    .metric-grid {
                        display: grid;
                        grid-template-columns: repeat(auto-fit, minmax(190px, 1fr));
                        gap: 14px;
                        margin-bottom: 20px;
                    }

                    .metric {
                        min-height: 118px;
                        padding: 16px;
                        border: 1px solid var(--border);
                        border-radius: 8px;
                        background: var(--panel);
                        display: flex;
                        flex-direction: column;
                        justify-content: space-between;
                        box-shadow: var(--shadow);
                    }

                    .metric-label {
                        color: var(--muted);
                        font-size: 0.78rem;
                        font-weight: 780;
                        text-transform: uppercase;
                        letter-spacing: 0;
                    }

                    .metric-value {
                        font-size: 2.15rem;
                        line-height: 1;
                        font-weight: 850;
                        color: var(--ink);
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
                        min-height: 40px;
                        padding: 9px 11px;
                        border: 1px solid var(--border-strong);
                        border-radius: 7px;
                        background: var(--field);
                        color: var(--ink);
                        font: inherit;
                        font-size: 0.94rem;
                        transition: border-color 140ms ease, box-shadow 140ms ease, background 140ms ease;
                    }

                    textarea {
                        min-height: 110px;
                        resize: vertical;
                        line-height: 1.45;
                    }

                    input:focus, select:focus, textarea:focus {
                        outline: 0;
                        border-color: var(--control);
                        background: #08111f;
                        box-shadow: 0 0 0 3px rgba(37, 99, 235, 0.12);
                    }

                    input[type=hidden] { display: none; }

                    input[type=checkbox] {
                        accent-color: var(--control);
                        cursor: pointer;
                    }

                    button, .button, .ghost-button {
                        min-height: 36px;
                        display: inline-flex;
                        align-items: center;
                        justify-content: center;
                        gap: 8px;
                        border-radius: 7px;
                        padding: 8px 12px;
                        font: inherit;
                        font-weight: 740;
                        font-size: 0.9rem;
                        text-decoration: none;
                        cursor: pointer;
                        transition: background 140ms ease, border-color 140ms ease, color 140ms ease, box-shadow 140ms ease;
                    }

                    button, .button {
                        border: 1px solid var(--control);
                        background: var(--control);
                        color: #ffffff;
                        box-shadow: 0 1px 2px rgba(15, 23, 42, 0.08);
                    }

                    button:hover, .button:hover {
                        background: var(--control-hover);
                        border-color: var(--control-hover);
                    }

                    .ghost-button {
                        border: 1px solid var(--border-strong);
                        background: #0a0f16;
                        color: var(--ink-soft);
                    }

                    .ghost-button:hover {
                        border-color: var(--control);
                        color: #bfdbfe;
                        background: #0f2a4a;
                    }

                    button.danger, .danger {
                        border-color: var(--danger);
                        background: var(--danger);
                        color: #ffffff;
                        box-shadow: none;
                    }

                    button.danger:hover, .danger:hover {
                        background: var(--danger-hover);
                        border-color: var(--danger-hover);
                    }

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
                        border: 1px solid #fda29b;
                    }

                    .fine-print {
                        margin: 14px 0 0;
                        font-size: 0.86rem;
                    }

                    .table-wrap {
                        width: 100%;
                        max-width: 100%;
                        overflow-x: auto;
                        border: 1px solid var(--border);
                        border-radius: 8px;
                        background: var(--panel);
                    }

                    table.data-table {
                        width: 100%;
                        min-width: 760px;
                        border-collapse: collapse;
                    }

                    .data-table th,
                    .data-table td {
                        padding: 12px 14px;
                        border-bottom: 1px solid var(--border);
                        text-align: left;
                        vertical-align: top;
                    }

                    .data-table th {
                        color: #94a3b8;
                        background: var(--panel-muted);
                        font-size: 0.76rem;
                        font-weight: 820;
                        text-transform: uppercase;
                        letter-spacing: 0;
                    }

                    .data-table tr:last-child td { border-bottom: 0; }
                    .data-table tbody tr { transition: background 160ms ease, box-shadow 160ms ease; }
                    .data-table tbody tr:hover {
                        background: var(--row-hover);
                        box-shadow: inset 3px 0 0 var(--control);
                    }

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
                        font-family: ui-monospace, monospace;
                        font-size: 0.84rem;
                        line-height: 1.45;
                        word-break: break-all;
                        color: #93c5fd;
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

                    .asset-access-list {
                        display: grid;
                        gap: 10px;
                        margin-top: 14px;
                    }

                    .asset-access-item {
                        display: grid;
                        grid-template-columns: 22px minmax(0, 1fr);
                        gap: 10px;
                        align-items: start;
                        padding: 12px;
                        border: 1px solid var(--border);
                        border-radius: 8px;
                        background: var(--panel-muted);
                    }

                    .asset-access-item input {
                        width: 18px;
                        min-height: 18px;
                        margin-top: 2px;
                    }

                    .asset-access-item[hidden], [data-asset-access-list][hidden] {
                        display: none;
                    }

                    .import-summary {
                        display: grid;
                        gap: 8px;
                    }

                    .command-block {
                        display: grid;
                        gap: 8px;
                        margin-top: 8px;
                    }

                    .command-input {
                        font-family: ui-monospace, monospace;
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
                        background: var(--tag-bg);
                        color: var(--tag-ink);
                        text-decoration: none;
                    }

                    .tag:hover {
                        background: #dbe7f6;
                        color: #17202a;
                    }

                    .status-pill {
                        background: var(--secure-soft);
                        color: var(--secure);
                    }

                    .status-pill.neutral {
                        background: var(--panel-muted);
                        color: #94a3b8;
                        border: 1px solid var(--border);
                    }

                    .status-pill.danger {
                        background: var(--danger-soft);
                        color: var(--danger);
                        border: 1px solid #fda29b;
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
                        border: 1px solid var(--border);
                        border-radius: 8px;
                        background: var(--terminal);
                        padding: 14px;
                        color: #bfdbfe;
                    }

                    .dashboard-page,
                    .assets-page,
                    .audit-page {
                        display: grid;
                        gap: 18px;
                    }

                    .console-hero {
                        display: flex;
                        align-items: flex-start;
                        justify-content: space-between;
                        gap: 18px;
                        padding: 2px 0 4px;
                    }

                    .console-hero h2 {
                        margin: 0;
                        font-size: 1.5rem;
                        line-height: 1.12;
                    }

                    .console-hero p {
                        margin: 7px 0 0;
                        color: var(--muted);
                        line-height: 1.5;
                    }

                    .console-actions,
                    .status-row {
                        display: flex;
                        align-items: center;
                        flex-wrap: wrap;
                        gap: 9px;
                    }

                    .status-chip,
                    .command-chip {
                        display: inline-flex;
                        align-items: center;
                        gap: 8px;
                        min-height: 28px;
                        padding: 5px 10px;
                        border-radius: 999px;
                        border: 1px solid var(--border);
                        background: #0a0f16;
                        color: var(--ink-soft);
                        font-size: 0.82rem;
                        font-weight: 760;
                        white-space: nowrap;
                    }

                    .status-chip.good {
                        background: var(--secure-soft);
                        border-color: #14532d;
                        color: var(--secure);
                    }

                    .status-chip.warn {
                        background: var(--warn-soft);
                        border-color: #713f12;
                        color: var(--warn);
                    }

                    .status-chip.danger {
                        background: var(--danger-soft);
                        border-color: #7f1d1d;
                        color: var(--danger);
                    }

                    .status-dot.good { background: var(--console-green); }
                    .status-dot.warn { background: var(--warn); }
                    .status-dot.danger { background: var(--danger); }

                    .dashboard-grid,
                    .audit-grid {
                        display: grid;
                        grid-template-columns: minmax(0, 1fr) 320px;
                        gap: 18px;
                        align-items: start;
                    }

                    .panel-stack {
                        display: grid;
                        gap: 18px;
                        min-width: 0;
                    }

                    .metric {
                        background: #111827;
                    }

                    .metric-value {
                        font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
                    }

                    .metric-note strong {
                        color: var(--secure);
                        font-weight: 850;
                    }

                    .chart-bars {
                        height: 132px;
                        display: grid;
                        grid-template-columns: repeat(24, minmax(6px, 1fr));
                        align-items: end;
                        gap: 5px;
                    }

                    .chart-bars span {
                        min-height: 14px;
                        border-radius: 5px 5px 2px 2px;
                        background: linear-gradient(180deg, #60a5fa, #1d4ed8);
                        opacity: 0.86;
                    }

                    .heatmap {
                        display: grid;
                        grid-template-columns: repeat(12, 1fr);
                        gap: 5px;
                    }

                    .heatmap span {
                        aspect-ratio: 1;
                        min-width: 12px;
                        border-radius: 4px;
                        background: #0f1724;
                        border: 1px solid #172033;
                    }

                    .heatmap .level-1 { background: #0b2e1c; }
                    .heatmap .level-2 { background: #14532d; }
                    .heatmap .level-3 { background: #166534; }
                    .heatmap .level-4 { background: #22c55e; }

                    .posture-list,
                    .incident-list,
                    .feed-list {
                        display: grid;
                        gap: 10px;
                    }

                    .posture-item,
                    .incident-item,
                    .feed-item {
                        display: grid;
                        grid-template-columns: 10px minmax(0, 1fr) auto;
                        gap: 10px;
                        align-items: center;
                        padding: 10px 0;
                        border-bottom: 1px solid var(--border);
                    }

                    .posture-item:last-child,
                    .incident-item:last-child,
                    .feed-item:last-child {
                        border-bottom: 0;
                    }

                    .posture-item b,
                    .incident-item b,
                    .feed-item b {
                        color: var(--ink);
                        font-size: 0.9rem;
                    }

                    .posture-item span:last-child,
                    .incident-item span:last-child,
                    .feed-item span:last-child {
                        color: var(--muted);
                        font-size: 0.82rem;
                    }

                    .dashboard-activity-list,
                    .dashboard-event-list,
                    .coverage-list {
                        display: grid;
                        gap: 0;
                    }

                    .dashboard-activity-item,
                    .dashboard-event-item {
                        min-width: 0;
                        display: grid;
                        grid-template-columns: 10px minmax(0, 1.45fr) minmax(100px, 0.8fr) minmax(110px, auto);
                        gap: 12px;
                        align-items: center;
                        padding: 12px 0;
                        border-bottom: 1px solid var(--border);
                    }

                    .dashboard-event-item {
                        grid-template-columns: 10px minmax(0, 1fr) minmax(150px, 1fr) auto;
                    }

                    .dashboard-activity-item:last-child,
                    .dashboard-event-item:last-child {
                        border-bottom: 0;
                    }

                    .dashboard-activity-item b,
                    .dashboard-event-item b {
                        display: block;
                        overflow: hidden;
                        color: var(--ink);
                        font-size: 0.9rem;
                        text-overflow: ellipsis;
                        white-space: nowrap;
                    }

                    .dashboard-activity-item small,
                    .dashboard-event-item small,
                    .coverage-item small {
                        display: block;
                        margin-top: 3px;
                        color: var(--muted);
                        font-size: 0.78rem;
                    }

                    .dashboard-result {
                        display: grid;
                        justify-items: end;
                        gap: 4px;
                    }

                    .coverage-item {
                        padding: 12px 0;
                        border-bottom: 1px solid var(--border);
                    }

                    .coverage-item:last-child {
                        border-bottom: 0;
                    }

                    .coverage-label {
                        display: flex;
                        align-items: center;
                        justify-content: space-between;
                        gap: 12px;
                        color: var(--ink-soft);
                        font-size: 0.86rem;
                    }

                    .coverage-label strong {
                        color: var(--ink);
                        font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
                    }

                    .coverage-track {
                        height: 7px;
                        margin-top: 8px;
                        overflow: hidden;
                        border-radius: 999px;
                        background: #0a0f16;
                        border: 1px solid var(--border);
                    }

                    .coverage-track span {
                        display: block;
                        height: 100%;
                        border-radius: inherit;
                        background: var(--control);
                    }

                    .dashboard-panel-link {
                        color: #93c5fd;
                        font-size: 0.82rem;
                        font-weight: 750;
                        text-decoration: none;
                    }

                    .dashboard-panel-link:hover {
                        color: #ffffff;
                        text-decoration: underline;
                    }

                    .admin-access-list {
                        display: grid;
                        gap: 12px;
                    }

                    .admin-access-item {
                        min-width: 0;
                        display: grid;
                        grid-template-columns: minmax(180px, 1fr) minmax(260px, 1.5fr);
                        gap: 18px;
                        align-items: center;
                        padding: 16px;
                        border: 1px solid var(--border);
                        border-radius: 8px;
                        background: var(--panel-muted);
                    }

                    .admin-access-item form {
                        display: grid;
                        grid-template-columns: minmax(150px, 1fr) auto auto;
                        gap: 10px;
                        align-items: end;
                    }

                    .admin-access-item .field {
                        margin: 0;
                    }

                    .checkbox-line {
                        min-height: 40px;
                        display: inline-flex;
                        align-items: center;
                        gap: 8px;
                        white-space: nowrap;
                    }

                    .checkbox-line input[type=checkbox] {
                        width: 18px;
                        height: 18px;
                        min-height: 0;
                        flex: 0 0 auto;
                    }

                    .admin-access-item button {
                        width: auto;
                    }

                    .admin-person {
                        min-width: 0;
                    }

                    .admin-person b {
                        color: var(--ink);
                    }

                    .admin-person .mono {
                        display: block;
                        margin-top: 4px;
                    }

                    .terminal-strip {
                        display: flex;
                        align-items: center;
                        gap: 10px;
                        padding: 12px 14px;
                        border: 1px solid var(--border);
                        border-radius: 8px;
                        background: var(--terminal);
                        color: #bfdbfe;
                        font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
                        font-size: 0.84rem;
                        overflow-x: auto;
                    }

                    .assets-layout {
                        display: grid;
                        grid-template-columns: minmax(0, 1fr) 360px;
                        gap: 18px;
                        align-items: start;
                    }

                    .assets-layout > * {
                        min-width: 0;
                    }

                    .filter-console {
                        display: flex;
                        align-items: center;
                        justify-content: space-between;
                        gap: 12px;
                        flex-wrap: wrap;
                    }

                    .filter-console .filter-row {
                        flex: 1;
                        min-width: 260px;
                    }

                    .asset-form-panel {
                        position: sticky;
                        top: 106px;
                    }

                    .assets-page .panel-header {
                        flex-wrap: wrap;
                    }

                    .assets-page .data-table {
                        min-width: 660px;
                    }

                    .os-badge {
                        display: inline-grid;
                        place-items: center;
                        min-width: 34px;
                        min-height: 26px;
                        border-radius: 6px;
                        background: #122c4e;
                        color: #bfdbfe;
                        font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
                        font-size: 0.72rem;
                        font-weight: 850;
                    }

                    .audit-toolbar {
                        display: flex;
                        align-items: center;
                        justify-content: space-between;
                        flex-wrap: wrap;
                        gap: 10px;
                        padding: 12px;
                        border: 1px solid var(--border);
                        border-radius: 8px;
                        background: #0a0f16;
                    }

                    .audit-event {
                        font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
                        color: #bfdbfe;
                        font-size: 0.82rem;
                    }

                    .replay-box {
                        display: grid;
                        gap: 12px;
                        padding: 14px;
                        border: 1px solid var(--border);
                        border-radius: 8px;
                        background: var(--terminal);
                    }

                    .replay-progress {
                        height: 8px;
                        border-radius: 999px;
                        background: #111827;
                        overflow: hidden;
                    }

                    .replay-progress span {
                        display: block;
                        width: 46%;
                        height: 100%;
                        background: linear-gradient(90deg, #3b82f6, #22c55e);
                    }

                    .mobile-tabbar {
                        display: none;
                    }

                    @media (prefers-reduced-motion: reduce) {
                        *, *::before, *::after {
                            scroll-behavior: auto !important;
                            transition-duration: 0.01ms !important;
                            animation-duration: 0.01ms !important;
                            animation-iteration-count: 1 !important;
                        }
                    }

                    @media (max-width: 1080px) {
                        .app-frame { grid-template-columns: 1fr; }
                        .sidebar {
                            display: none;
                        }
                        .topbar {
                            position: static;
                            padding: 22px 20px 16px;
                        }
                        .workspace {
                            padding: 22px 20px 96px;
                        }
                        .dashboard-grid,
                        .audit-grid,
                        .assets-layout {
                            grid-template-columns: 1fr;
                        }
                        .asset-form-panel {
                            position: static;
                        }
                        .mobile-tabbar {
                            position: fixed;
                            left: 0;
                            right: 0;
                            bottom: 0;
                            z-index: 20;
                            display: grid;
                            grid-template-columns: repeat(4, minmax(0, 1fr));
                            gap: 1px;
                            padding: 8px 10px calc(8px + env(safe-area-inset-bottom));
                            border-top: 1px solid var(--border);
                            background: rgba(10, 15, 22, 0.96);
                            backdrop-filter: blur(16px);
                        }
                        .mobile-tab {
                            min-height: 48px;
                            display: grid;
                            place-items: center;
                            gap: 3px;
                            border-radius: 8px;
                            color: var(--muted);
                            text-decoration: none;
                            font-size: 0.72rem;
                            font-weight: 760;
                        }
                        .mobile-tab svg {
                            width: 18px;
                            height: 18px;
                        }
                        .mobile-tab.active {
                            background: #0f2a4a;
                            color: #bfdbfe;
                        }
                    }

                    @media (max-width: 560px) {
                        .topbar {
                            flex-direction: column;
                            align-items: stretch;
                        }
                        .ghost-button, .button, button { width: 100%; }
                        .panel { padding: 16px; }
                        .panel-header { flex-direction: column; }
                        .topbar h1 { font-size: 1.65rem; }
                        .console-hero {
                            flex-direction: column;
                        }
                        .console-actions,
                        .status-row,
                        .filter-console,
                        .audit-toolbar {
                            align-items: stretch;
                            flex-direction: column;
                        }
                        .status-chip,
                        .command-chip {
                            justify-content: center;
                        }
                        .metric-grid {
                            grid-template-columns: repeat(2, minmax(0, 1fr));
                        }
                        .metric-value {
                            font-size: 1.8rem;
                        }
                        .dashboard-activity-item,
                        .dashboard-event-item {
                            grid-template-columns: 10px minmax(0, 1fr) auto;
                        }
                        .dashboard-activity-item > :nth-child(3),
                        .dashboard-event-item > :nth-child(3) {
                            grid-column: 2;
                        }
                        .dashboard-result {
                            grid-column: 3;
                            grid-row: 1 / span 2;
                        }
                        .admin-access-item,
                        .admin-access-item form {
                            grid-template-columns: 1fr;
                        }
                        .admin-access-item button {
                            width: 100%;
                        }
                        .heatmap {
                            grid-template-columns: repeat(8, 1fr);
                        }
                    }
                    "#
                }
                style {
                    (PreEscaped(include_str!("release_a_assets.css")))
                    (PreEscaped(include_str!("release_b_credentials_trust.css")))
                }
            }
            body class="admin-shell" data-theme="operator" {
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
                            (nav_link("/", t.nav_overview, ICON_OVERVIEW, active == "overview"))
                            (nav_link("/assets", t.nav_assets, ICON_ASSETS, active == "assets"))
                            (nav_link("/credentials", t.nav_credentials, ICON_CREDENTIALS, active == "credentials"))
                            (nav_link("/keys", t.nav_keys, ICON_KEYS, active == "keys"))
                            (nav_link("/known-hosts", t.nav_known_hosts, ICON_KNOWN_HOSTS, active == "known-hosts"))
                            (nav_link("/sessions", t.nav_sessions, ICON_SESSIONS, active == "sessions"))
                            (nav_link("/import", t.nav_import_export, ICON_IMPORT, active == "import"))
                            (nav_link("/settings", t.nav_settings, ICON_SETTINGS, active == "settings"))
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
                        nav.mobile-tabbar aria-label=(t.nav_primary) {
                            (mobile_nav_link("/", t.nav_overview, ICON_OVERVIEW, active == "overview"))
                            (mobile_nav_link("/assets", t.nav_assets, ICON_ASSETS, active == "assets"))
                            (mobile_nav_link("/sessions", t.nav_sessions, ICON_SESSIONS, active == "sessions"))
                            (mobile_nav_link("/settings", t.nav_settings, ICON_SETTINGS, active == "settings"))
                        }
                    }
                }
            }
        }
    }
}

fn login_layout(title: &str, t: &L10n, body_content: Markup) -> Markup {
    let alternate = t.locale.alternate();
    let language_href = language_switch_href(alternate, "login");

    html! {
        (DOCTYPE)
        html lang=(t.locale.code()) {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) " - " (t.app_title) }
                style {
                    (PreEscaped(include_str!("release_a_login.css")))
                }
            }
            body class="admin-shell login-shell" data-theme="operator" {
                div.login-page {
                    header.login-header {
                        div.login-brand aria-label=(t.app_title) {
                            div.login-brand-mark aria-hidden="true" { "H" }
                            div {
                                strong { "Hop" }
                                span { (t.admin_console) }
                            }
                        }
                        a.login-language href=(language_href) {
                            span { (t.language_label) ": " (t.locale.label()) }
                            strong { (t.switch_language_to) " " (alternate.label()) }
                        }
                    }
                    main.login-main {
                        div.login-context {
                            span.status-dot aria-hidden="true" {}
                            div {
                                strong { (t.loopback_admin) }
                                p { (t.loopback_note) }
                            }
                        }
                        (body_content)
                    }
                }
            }
        }
    }
}

pub fn login(t: &L10n, error: Option<&str>, show_username: bool, username: Option<&str>) -> Markup {
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
                            p { (if show_username { t.login_team_intro } else { t.login_intro }) }
                        }
                    }
                    @if let Some(error) = error {
                        p.error-message id="login-error" role="alert" { (error) }
                    }
                    form method="post" action="/login" {
                        @if show_username {
                            label.field {
                                (t.login_username)
                                input
                                    id="login-username"
                                    name="username"
                                    value=(username.unwrap_or(""))
                                    autocomplete="username"
                                    autofocus
                                    required;
                            }
                        }
                        label.field {
                            (t.login_password)
                            @if error.is_some() {
                                input
                                    id="login-password"
                                    type="password"
                                    name="password"
                                    autocomplete="current-password"
                                    aria-describedby="login-error"
                                    aria-invalid="true"
                                    autofocus[!show_username]
                                    required;
                            } @else {
                                input
                                    id="login-password"
                                    type="password"
                                    name="password"
                                    autocomplete="current-password"
                                    autofocus[!show_username]
                                    required;
                            }
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

pub fn permission_denied(t: &L10n, task: &str, access_profile: &str) -> Markup {
    layout(
        t.permission_denied_title,
        "",
        t,
        html! {
            div.page-intro {
                h2 { (t.permission_denied_heading) }
                p { (t.permission_denied_intro) }
            }
            section.panel {
                div.posture-list {
                    div.posture-item {
                        span.status-dot.warn {}
                        b { (t.permission_current_access) }
                        span { (admin_profile_label(t, access_profile)) }
                    }
                    div.posture-item {
                        span.status-dot.good {}
                        b { (t.permission_owner_action) }
                        a.dashboard-panel-link href="/settings" { (t.nav_settings) }
                    }
                }
                pre {
                    @match t.locale {
                        Locale::En => { (t.permission_request_prefix) " " (task) "." }
                        Locale::Zh => { (t.permission_request_prefix) (task) "。" }
                    }
                }
            }
        },
    )
}

fn admin_profile_label<'a>(t: &'a L10n, profile: &str) -> &'a str {
    match profile {
        ADMIN_PROFILE_OWNER => t.admin_profile_owner,
        ADMIN_PROFILE_OPERATOR => t.admin_profile_operator,
        ADMIN_PROFILE_VIEWER => t.admin_profile_viewer,
        _ => t.none,
    }
}

fn admin_profile_intro<'a>(t: &'a L10n, profile: &str) -> &'a str {
    match profile {
        ADMIN_PROFILE_OWNER => t.admin_profile_owner_intro,
        ADMIN_PROFILE_OPERATOR => t.admin_profile_operator_intro,
        ADMIN_PROFILE_VIEWER => t.admin_profile_viewer_intro,
        _ => t.none,
    }
}

pub fn settings(
    t: &L10n,
    current_admin: &AdminUser,
    admins: &[AdminUser],
    csrf_token: &str,
    error: Option<&str>,
    can_manage_admins: bool,
) -> Markup {
    let active_admin_count = admins.iter().filter(|admin| admin.is_active).count();
    let active_owner_count = admins
        .iter()
        .filter(|admin| admin.is_active && admin.access_profile == ADMIN_PROFILE_OWNER)
        .count();
    layout(
        t.settings_title,
        "settings",
        t,
        html! {
            div.page-intro {
                h2 { (t.settings_heading) }
                p { (t.settings_intro) }
            }
            @if let Some(error) = error {
                p.error-message role="alert" { (error) }
            }
            @if current_admin.must_change_password {
                p.error-message role="status" { (t.admin_must_change_password) }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.admin_password_heading) }
                        p { (t.admin_password_intro) }
                    }
                }
                form method="post" action="/settings" {
                    (csrf_field(csrf_token))
                    div.grid {
                        label.field {
                            (t.current_password)
                            input type="password" name="current_password" autocomplete="current-password" required;
                        }
                        label.field {
                            (t.new_password)
                            input type="password" name="new_password" autocomplete="new-password" minlength="12" required;
                        }
                        label.field {
                            (t.confirm_password)
                            input type="password" name="confirm_password" autocomplete="new-password" minlength="12" required;
                        }
                    }
                    div.button-row {
                        button type="submit" { (t.change_password) }
                    }
                }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.admin_access_heading) }
                        p { (t.admin_access_intro) }
                    }
                    div.status-row {
                        span.status-chip { (active_admin_count) " · " (t.admin_access_heading) }
                        @if can_manage_admins {
                            button
                                id="open-add-admin"
                                type="button"
                                aria-haspopup="dialog"
                                aria-controls="add-admin-dialog"
                                onclick="document.getElementById('add-admin-dialog').showModal()" {
                                (t.admin_add_action)
                            }
                        }
                    }
                }
                p.fine-print {
                    (if active_admin_count == 1 { t.admin_single_mode } else { t.admin_team_mode })
                }
                div.panel-header {
                    div {
                        h3 { (t.admin_existing_heading) }
                        p { (t.admin_existing_intro) }
                    }
                }
                div.admin-access-list {
                    @for admin in admins {
                        @let is_current = admin.id == current_admin.id;
                        @let is_last_owner = admin.is_active
                            && admin.access_profile == ADMIN_PROFILE_OWNER
                            && active_owner_count == 1;
                        div.admin-access-item {
                            div.admin-person {
                                b {
                                    (admin.display_name)
                                    @if is_current {
                                        " · " (t.admin_current_user)
                                    }
                                }
                                span.mono { "@" (admin.username) }
                                span.subtle {
                                    (t.admin_last_login) ": "
                                    (admin.last_login_at.as_deref().unwrap_or("-"))
                                }
                                @if admin.must_change_password {
                                    span.status-chip.warn { (t.admin_must_change_password) }
                                }
                            }
                            @if can_manage_admins && !is_last_owner {
                                form method="post" action=(format!("/settings/admins/{}/access", admin.id)) {
                                    (csrf_field(csrf_token))
                                    label.field {
                                        (t.admin_access_level)
                                        select name="access_profile" {
                                            (admin_profile_options(t, &admin.access_profile))
                                        }
                                    }
                                    label.checkbox-line {
                                        input type="checkbox" name="is_active" value="yes" checked[admin.is_active];
                                        (t.admin_login_active)
                                    }
                                    button type="submit" { (t.admin_save_access) }
                                }
                            } @else {
                                div.primary-cell {
                                    span.status-pill {
                                        (admin_profile_label(t, &admin.access_profile))
                                    }
                                    span.subtle {
                                        (admin_profile_intro(t, &admin.access_profile))
                                    }
                                    @if is_last_owner {
                                        span.subtle { (t.admin_last_owner_note) }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            @if can_manage_admins {
                dialog.asset-drawer
                    id="add-admin-dialog"
                    aria-labelledby="add-admin-title"
                    onclose="document.getElementById('open-add-admin').focus()" {
                    div.drawer-frame {
                        div.drawer-header {
                            div {
                                span.status-chip { (t.admin_add_action) }
                                h2 id="add-admin-title" { (t.admin_add_heading) }
                                p { (t.admin_add_intro) }
                            }
                            form method="dialog" {
                                button.ghost-button type="submit" { (t.close) }
                            }
                        }
                        form method="post" action="/settings/admins" {
                            (csrf_field(csrf_token))
                            div.grid {
                                label.field {
                                    (t.admin_display_name)
                                    input name="display_name" autocomplete="name" required autofocus;
                                }
                                label.field {
                                    (t.admin_username)
                                    input name="username" autocomplete="off" pattern="[A-Za-z0-9_.-]+" required;
                                }
                                label.field.field-wide {
                                    (t.admin_temporary_password)
                                    input type="password" name="temporary_password" autocomplete="new-password" minlength="12" required;
                                    span.subtle { (t.admin_temporary_password_note) }
                                }
                                label.field.field-wide {
                                    (t.admin_access_level)
                                    select name="access_profile" {
                                        option value=(ADMIN_PROFILE_OPERATOR) selected {
                                            (t.admin_profile_operator) " · " (t.admin_recommended)
                                        }
                                        option value=(ADMIN_PROFILE_VIEWER) { (t.admin_profile_viewer) }
                                        option value=(ADMIN_PROFILE_OWNER) { (t.admin_profile_owner) }
                                    }
                                }
                            }
                            details {
                                summary { (t.admin_access_level) }
                                p { b { (t.admin_profile_operator) } " — " (t.admin_profile_operator_intro) }
                                p { b { (t.admin_profile_viewer) } " — " (t.admin_profile_viewer_intro) }
                                p { b { (t.admin_profile_owner) } " — " (t.admin_profile_owner_intro) }
                            }
                            div.button-row {
                                button type="submit" { (t.admin_add_action) }
                            }
                        }
                    }
                }
            }
        },
    )
}

fn admin_profile_options(t: &L10n, selected: &str) -> Markup {
    html! {
        option value=(ADMIN_PROFILE_OPERATOR) selected[selected == ADMIN_PROFILE_OPERATOR] {
            (t.admin_profile_operator)
        }
        option value=(ADMIN_PROFILE_VIEWER) selected[selected == ADMIN_PROFILE_VIEWER] {
            (t.admin_profile_viewer)
        }
        option value=(ADMIN_PROFILE_OWNER) selected[selected == ADMIN_PROFILE_OWNER] {
            (t.admin_profile_owner)
        }
    }
}

#[derive(Debug, Clone)]
pub struct DashboardGateway {
    pub admin_bind: String,
    pub ssh_bind: String,
    pub version: String,
    pub started_at: String,
    pub uptime_seconds: u64,
    pub admin_reachable: bool,
    pub ssh_reachable: bool,
    pub database_healthy: bool,
}

#[derive(Debug, Clone)]
pub struct DashboardData {
    pub gateway: DashboardGateway,
    pub assets: Vec<Asset>,
    pub credentials: Vec<Credential>,
    pub keys: Vec<AuthorizedKey>,
    pub known_hosts: Vec<KnownHost>,
    pub asset_health: Vec<AssetHealth>,
    pub recent_sessions: Vec<Session>,
    pub sessions_24h: i64,
    pub recent_admin_events: Vec<AuditEvent>,
    pub source_errors: Vec<String>,
}

pub fn overview(t: &L10n, data: &DashboardData) -> Markup {
    let ssh_assets: Vec<&Asset> = data
        .assets
        .iter()
        .filter(|asset| asset.protocol == ASSET_PROTOCOL_SSH)
        .collect();
    let managed_ssh_assets = ssh_assets
        .iter()
        .filter(|asset| asset.credential_id.is_some())
        .count();
    let active_keys = data.keys.iter().filter(|key| key.is_active).count();
    let restricted_keys = data
        .keys
        .iter()
        .filter(|key| key.is_active && key.asset_access_mode == AssetAccessMode::Restricted)
        .count();
    let trusted_ssh_assets = ssh_assets
        .iter()
        .filter(|asset| {
            data.known_hosts
                .iter()
                .any(|host| host.hostname == asset.hostname && host.port == asset.port)
        })
        .count();
    let healthy_assets = data
        .assets
        .iter()
        .filter(|asset| asset_health_status(data, &asset.id) == ASSET_HEALTH_HEALTHY)
        .count();
    let failed_assets = data
        .assets
        .iter()
        .filter(|asset| asset_health_status(data, &asset.id) == ASSET_HEALTH_FAILED)
        .count();
    let unknown_assets = data.assets.len() - healthy_assets - failed_assets;
    let unmanaged_ssh_assets = ssh_assets.len().saturating_sub(managed_ssh_assets);
    let risk_count = usize::from(!data.source_errors.is_empty())
        + usize::from(failed_assets > 0)
        + usize::from(unknown_assets > 0)
        + usize::from(unmanaged_ssh_assets > 0)
        + usize::from(!data.assets.is_empty() && active_keys == 0);
    let gateway_operational = data.gateway.admin_reachable
        && data.gateway.ssh_reachable
        && data.gateway.database_healthy
        && data.source_errors.is_empty()
        && failed_assets == 0;
    let managed_coverage = coverage_percent(managed_ssh_assets, ssh_assets.len());
    let restricted_coverage = coverage_percent(restricted_keys, active_keys);
    let trust_coverage = coverage_percent(trusted_ssh_assets, ssh_assets.len());

    layout(
        t.overview_title,
        "overview",
        t,
        html! {
            div.dashboard-page {
                div.console-hero {
                    div {
                        h2 { (t.overview_heading) }
                        p { (t.overview_intro) }
                    }
                    div.console-actions {
                        @if gateway_operational {
                            span.status-chip.good { span.status-dot.good {} (t.overview_status_operational) }
                        } @else {
                            span.status-chip.warn { span.status-dot.warn {} (t.overview_status_attention) }
                        }
                        a.button href="/assets" { (t.assets_add_heading) }
                    }
                }
                div.metric-grid {
                    div.metric {
                        span.metric-label { (t.overview_assets_label) }
                        strong.metric-value { (data.assets.len()) }
                        span.metric-note {
                            (healthy_assets) " " (t.overview_health_healthy) " · "
                            (failed_assets) " " (t.overview_health_failed) " · "
                            (unknown_assets) " " (t.overview_health_unknown)
                        }
                    }
                    div.metric {
                        span.metric-label { (t.overview_sessions_24h_label) }
                        strong.metric-value { (data.sessions_24h) }
                        span.metric-note { (t.overview_sessions_24h_note) }
                    }
                    div.metric {
                        span.metric-label { (t.overview_active_keys_label) }
                        strong.metric-value { (active_keys) }
                        span.metric-note { (t.overview_active_keys_note) }
                    }
                    div.metric {
                        span.metric-label { (t.overview_managed_coverage_label) }
                        strong.metric-value { (managed_coverage) "%" }
                        span.metric-note {
                            (managed_ssh_assets) " / " (ssh_assets.len()) " "
                            (t.overview_managed_coverage_suffix)
                        }
                    }
                }
                div.dashboard-grid {
                    div.panel-stack {
                        section.panel {
                            div.panel-header {
                                div {
                                    h2 { (t.overview_recent_access_heading) }
                                    p { (t.overview_recent_access_intro) }
                                }
                                a.dashboard-panel-link href="/sessions" { (t.overview_view_all) }
                            }
                            div.dashboard-activity-list {
                                @if data.recent_sessions.is_empty() {
                                    p.fine-print { (t.overview_no_recent_access) }
                                }
                                @for session in &data.recent_sessions {
                                    div.dashboard-activity-item {
                                        @if session.status == "failed" {
                                            span.status-dot.danger {}
                                        } @else if session.status == "ok" {
                                            span.status-dot.good {}
                                        } @else {
                                            span.status-dot.warn {}
                                        }
                                        div {
                                            b { (session.key_name.as_deref().unwrap_or("-")) }
                                            small {
                                                (session.asset_name.as_deref().unwrap_or("-"))
                                                @if let Some(started_at) = &session.started_at {
                                                    " · " (started_at)
                                                }
                                            }
                                        }
                                        div {
                                            b.mono { (session.mode) }
                                            small.mono {
                                                @if let Some(host) = &session.target_host {
                                                    (host) ":" (session.target_port.unwrap_or_default())
                                                } @else {
                                                    "-"
                                                }
                                            }
                                        }
                                        div.dashboard-result {
                                            @if session.status == "failed" {
                                                span.status-pill.danger { (session.status) }
                                            } @else if session.status == "ok" {
                                                span.status-pill { (session.status) }
                                            } @else {
                                                span.status-pill.neutral { (session.status) }
                                            }
                                            small.mono { (session_duration_label(session)) }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    div.panel-stack {
                        section.panel {
                            div.panel-header {
                                div {
                                    h2 { (t.overview_risk_heading) }
                                    p { (t.overview_risk_intro) }
                                }
                                @if risk_count > 0 {
                                    span.status-chip.warn { (risk_count) }
                                }
                            }
                            div.incident-list {
                                @if !data.source_errors.is_empty() {
                                    div.incident-item {
                                        span.status-dot.danger {}
                                        b { (t.overview_source_unavailable) }
                                        span { (data.source_errors.join(", ")) }
                                    }
                                }
                                @if failed_assets > 0 {
                                    div.incident-item {
                                        span.status-dot.danger {}
                                        b { (t.overview_failed_assets_risk) }
                                        a.dashboard-panel-link href="/assets" { (failed_assets) }
                                    }
                                }
                                @if unknown_assets > 0 {
                                    div.incident-item {
                                        span.status-dot.warn {}
                                        b { (t.overview_unknown_assets_risk) }
                                        a.dashboard-panel-link href="/assets" { (unknown_assets) }
                                    }
                                }
                                @if unmanaged_ssh_assets > 0 {
                                    div.incident-item {
                                        span.status-dot.warn {}
                                        b { (t.overview_unmanaged_assets_risk) }
                                        a.dashboard-panel-link href="/credentials" { (unmanaged_ssh_assets) }
                                    }
                                }
                                @if !data.assets.is_empty() && active_keys == 0 {
                                    div.incident-item {
                                        span.status-dot.warn {}
                                        b { (t.overview_no_active_keys_risk) }
                                        a.dashboard-panel-link href="/keys" { (t.nav_keys) }
                                    }
                                }
                                @if risk_count == 0 {
                                    div.incident-item {
                                    span.status-dot.good {}
                                        b { (t.overview_no_risks) }
                                        span { "✓" }
                                    }
                                }
                            }
                        }
                    }
                }
                div.dashboard-grid {
                    section.panel {
                        div.panel-header {
                            div {
                                h2 { (t.overview_gateway_heading) }
                                p { (t.overview_gateway_intro) }
                            }
                        }
                        div.posture-list {
                            (gateway_posture_row(t.overview_admin_endpoint, &data.gateway.admin_bind, data.gateway.admin_reachable, t))
                            (gateway_posture_row(t.overview_ssh_endpoint, &data.gateway.ssh_bind, data.gateway.ssh_reachable, t))
                            (gateway_posture_row(t.overview_database, "SQLite", data.gateway.database_healthy, t))
                            div.posture-item {
                                span.status-dot.good {}
                                b { (t.overview_version) }
                                span.mono { "v" (data.gateway.version) }
                            }
                            div.posture-item {
                                span.status-dot.good {}
                                b { (t.overview_started) }
                                span.mono { (data.gateway.started_at) }
                            }
                            div.posture-item {
                                span.status-dot.good {}
                                b { (t.overview_uptime) }
                                span.mono { (uptime_label(data.gateway.uptime_seconds)) }
                            }
                        }
                    }
                    section.panel {
                        div.panel-header {
                            div {
                                h2 { (t.overview_coverage_heading) }
                                p { (t.overview_coverage_intro) }
                            }
                            span.status-chip { (data.credentials.len()) " " (t.credentials_title) }
                        }
                        div.coverage-list {
                            (coverage_item(
                                t.overview_credential_coverage,
                                managed_ssh_assets,
                                ssh_assets.len(),
                                managed_coverage,
                            ))
                            (coverage_item(
                                t.overview_restricted_coverage,
                                restricted_keys,
                                active_keys,
                                restricted_coverage,
                            ))
                            (coverage_item(
                                t.overview_trust_coverage,
                                trusted_ssh_assets,
                                ssh_assets.len(),
                                trust_coverage,
                            ))
                        }
                    }
                }
                section.panel {
                    div.panel-header {
                        div {
                            h2 { (t.overview_recent_changes_heading) }
                            p { (t.overview_recent_changes_intro) }
                        }
                        a.dashboard-panel-link href="/sessions" { (t.overview_view_all) }
                    }
                    div.dashboard-event-list {
                        @if data.recent_admin_events.is_empty() {
                            p.fine-print { (t.overview_no_recent_changes) }
                        }
                        @for event in &data.recent_admin_events {
                            div.dashboard-event-item {
                                @if event.result == "failure" {
                                    span.status-dot.danger {}
                                } @else {
                                    span.status-dot.good {}
                                }
                                div {
                                    b { (event.actor_label) }
                                    small { (event.occurred_at.as_deref().unwrap_or("-")) }
                                }
                                div {
                                    b.mono { (event.action) }
                                    small {
                                        (event.target_label.as_deref().unwrap_or(event.target_type.as_str()))
                                    }
                                }
                                div.dashboard-result {
                                    @if event.result == "failure" {
                                        span.status-pill.danger { (event.result) }
                                    } @else {
                                        span.status-pill { (event.result) }
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

fn asset_health_status<'a>(data: &'a DashboardData, asset_id: &str) -> &'a str {
    data.asset_health
        .iter()
        .find(|health| health.asset_id == asset_id)
        .map(|health| health.status.as_str())
        .unwrap_or(ASSET_HEALTH_UNKNOWN)
}

fn coverage_percent(numerator: usize, denominator: usize) -> usize {
    numerator
        .saturating_mul(100)
        .checked_div(denominator)
        .unwrap_or(0)
}

fn coverage_item(label: &str, numerator: usize, denominator: usize, percent: usize) -> Markup {
    html! {
        div.coverage-item {
            div.coverage-label {
                span { (label) }
                strong { (percent) "%" }
            }
            div.coverage-track aria-hidden="true" {
                span style=(format!("width: {percent}%")) {}
            }
            small { (numerator) " / " (denominator) }
        }
    }
}

fn gateway_posture_row(label: &str, detail: &str, healthy: bool, t: &L10n) -> Markup {
    html! {
        div.posture-item {
            @if healthy {
                span.status-dot.good {}
            } @else {
                span.status-dot.danger {}
            }
            b { (label) }
            span {
                span.mono { (detail) }
                " · "
                @if healthy {
                    (t.overview_reachable)
                } @else {
                    (t.overview_unreachable)
                }
            }
        }
    }
}

fn uptime_label(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = seconds % 86_400 / 3_600;
    let minutes = seconds % 3_600 / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

fn session_duration_label(session: &Session) -> String {
    let (Some(started_at), Some(ended_at)) = (&session.started_at, &session.ended_at) else {
        return "—".to_string();
    };
    let (Some(started_at), Some(ended_at)) =
        (parse_timestamp(started_at), parse_timestamp(ended_at))
    else {
        return "—".to_string();
    };
    let seconds = (ended_at - started_at).num_seconds().max(0);
    if seconds >= 3_600 {
        format!("{}h {}m", seconds / 3_600, seconds % 3_600 / 60)
    } else if seconds >= 60 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}s")
    }
}

fn parse_timestamp(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&chrono::Utc))
        .ok()
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|timestamp| timestamp.and_utc())
        })
}

#[allow(clippy::too_many_arguments)]
pub fn assets(
    t: &L10n,
    items: &[Asset],
    credentials: &[Credential],
    csrf_token: &str,
    selected_tag: Option<&str>,
    search_query: Option<&str>,
    all_tags: &[String],
    ssh_port: u16,
    can_manage: bool,
) -> Markup {
    let search_query = search_query
        .map(str::trim)
        .filter(|query| !query.is_empty());
    let has_filters = selected_tag.is_some() || search_query.is_some();
    let return_to = assets_filter_href(selected_tag, search_query);

    layout(
        t.assets_title,
        "assets",
        t,
        html! {
            div.assets-page {
                div.console-hero {
                    div {
                        p { (t.assets_intro) }
                    }
                    div.console-actions {
                        span.status-chip.good { span.status-dot.good {} (items.len()) " " (t.assets_count_suffix) }
                        @if can_manage {
                            button
                                id="open-add-asset"
                                type="button"
                                aria-haspopup="dialog"
                                aria-controls="add-asset-dialog"
                                onclick="document.getElementById('add-asset-dialog').showModal()" {
                                (t.assets_add_heading)
                            }
                        }
                    }
                }
                section.panel.assets-toolbar aria-label=(t.assets_filter_heading) {
                    div.assets-toolbar-main {
                        form.assets-search method="get" action="/assets" {
                            @if let Some(tag) = selected_tag {
                                input type="hidden" name="tag" value=(tag);
                            }
                            div.assets-search-field {
                                label.field for="asset-search-query" { (t.assets_search_label) }
                                div.assets-search-row {
                                    input
                                        id="asset-search-query"
                                        type="search"
                                        name="q"
                                        value=(search_query.unwrap_or(""))
                                        placeholder=(t.assets_search_label);
                                    button type="submit" { (t.assets_search_action) }
                                    @if has_filters {
                                        a.ghost-button href="/assets" { (t.assets_clear_filters) }
                                    }
                                }
                            }
                        }
                        div.assets-filter-group {
                            div {
                                strong { (t.assets_filter_heading) }
                                p.fine-print { (t.assets_filter_intro) }
                            }
                            div.filter-row {
                                a
                                    class=(if selected_tag.is_none() { "button" } else { "ghost-button" })
                                    href=(assets_filter_href(None, search_query)) {
                                    (t.assets_filter_all)
                                }
                                @for tag in all_tags {
                                    a
                                        class=(if selected_tag == Some(tag.as_str()) { "button" } else { "ghost-button" })
                                        href=(assets_filter_href(Some(tag), search_query)) {
                                        (tag)
                                    }
                                }
                            }
                        }
                    }
                    div.assets-secondary-actions {
                        span.fine-print { (t.assets_export_intro) }
                        span.command-chip { (t.assets_export_heading) }
                        a.ghost-button href="/assets/export?format=csv" { (t.export_csv) }
                        a.ghost-button href="/assets/export?format=json" { (t.export_json) }
                        @if can_manage {
                            a.ghost-button href="/import" { (t.import_open) }
                        }
                    }
                }
                section.panel.assets-inventory {
                    div.panel-header {
                        div {
                            h2 { (t.assets_existing_heading) }
                            p { (t.assets_existing_intro) }
                        }
                        span.status-chip.good { (items.len()) " " (t.assets_count_suffix) }
                    }
                    form
                        method="post"
                        action="/assets/bulk-tags"
                        data-assets-form
                        onchange="if(event.target.matches('input[name=asset_ids]'))window.syncAssetBulkControls?.()" {
                        (csrf_field(csrf_token))
                        div.table-wrap {
                            table.data-table {
                                thead {
                                    tr {
                                        @if can_manage {
                                            th.checkbox-cell {}
                                        }
                                        th { (t.field_hostname) }
                                        th { (t.field_protocol) }
                                        th { (t.target_column) }
                                        th { (t.field_tags) }
                                        th { (t.field_credential) }
                                        @if can_manage {
                                            th { (t.field_action) }
                                        }
                                    }
                                }
                                tbody {
                                    @if items.is_empty() {
                                        tr.empty-row {
                                            td colspan=(if can_manage { "7" } else { "5" }) {
                                                div.assets-empty-state {
                                                    @if has_filters {
                                                        strong { (t.assets_filter_heading) }
                                                        span { (t.assets_filter_intro) }
                                                        a.ghost-button href="/assets" { (t.assets_filter_all) }
                                                    } @else {
                                                        strong { (t.no_assets) }
                                                        @if can_manage {
                                                            button
                                                                type="button"
                                                                onclick="document.getElementById('open-add-asset').click()" {
                                                                (t.assets_add_heading)
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    @for asset in items {
                                        tr {
                                            @if can_manage {
                                                td.checkbox-cell {
                                                    input
                                                        type="checkbox"
                                                        name="asset_ids"
                                                        value=(asset.id)
                                                        aria-label=(format!("{}: {}", t.assets_bulk_heading, asset.name));
                                                }
                                            }
                                            td {
                                                div.primary-cell {
                                                    (asset.name)
                                                    @if let Some(description) = &asset.description {
                                                        span.subtle { (description) }
                                                    } @else {
                                                        span.subtle { (t.asset_activity_placeholder) }
                                                    }
                                                    @if let Some(command) = asset_tunnel_command(asset, ssh_port) {
                                                        span.subtle.mono { (command) }
                                                    }
                                                }
                                            }
                                            td { span.os-badge { (asset_protocol_label(t, asset_kind(asset))) } }
                                            td.target-cell {
                                                @let target = format!("{}:{}", asset.hostname, asset.port);
                                                div.target-copy-group {
                                                    code
                                                        class="target-address"
                                                        tabindex="0"
                                                        title=(target) {
                                                        (target)
                                                    }
                                                    button
                                                        class="target-copy-button ghost-button"
                                                        type="button"
                                                        data-copy-value=(target)
                                                        data-copy-default=(t.asset_copy_target)
                                                        data-copy-success=(t.asset_copy_success)
                                                        data-copy-failed=(t.asset_copy_failed)
                                                        aria-label=(format!("{}: {}", t.asset_copy_target, target))
                                                        aria-live="polite"
                                                        onclick="window.copyAssetTarget(this)" {
                                                        (t.asset_copy_target)
                                                    }
                                                }
                                            }
                                            td {
                                                div.tag-list {
                                                    @if asset.tags.is_empty() {
                                                        span.status-pill.neutral { (t.untagged) }
                                                    }
                                                    @for tag in &asset.tags {
                                                        a.tag href=(assets_filter_href(Some(tag), search_query)) { (tag) }
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
                                            @if can_manage {
                                                td {
                                                    div.action-row {
                                                        a
                                                            id=(format!("edit-asset-{}", asset.id))
                                                            class="ghost-button"
                                                            href=(format!(
                                                                "/assets/{}/edit?return_to={}",
                                                                asset.id,
                                                                url_query_value(&return_to)
                                                            ))
                                                            aria-haspopup="dialog"
                                                            aria-controls="edit-asset-dialog"
                                                            data-asset-id=(asset.id)
                                                            data-asset-name=(asset.name)
                                                            data-asset-protocol=(asset_kind(asset))
                                                            data-asset-hostname=(asset.hostname)
                                                            data-asset-port=(asset.port)
                                                            data-asset-tags=(asset.tags.join(", "))
                                                            data-asset-credential=(asset.credential_id.as_deref().unwrap_or(""))
                                                            data-asset-description=(asset.description.as_deref().unwrap_or(""))
                                                            data-return-to=(return_to)
                                                            onclick="window.openAssetEditor(this);return false" {
                                                            (t.edit)
                                                        }
                                                        button class="danger" type="submit" formaction=(format!("/assets/{}/delete", asset.id)) { (t.delete) }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        @if can_manage {
                            div.assets-bulk-bar data-assets-bulk-controls hidden {
                                div.assets-bulk-copy {
                                    strong { (t.assets_bulk_heading) }
                                    span
                                        class="status-chip"
                                        data-assets-selected-count
                                        data-suffix=(t.assets_count_suffix)
                                        aria-live="polite" {}
                                    p.fine-print { (t.assets_bulk_intro) }
                                }
                                label.field {
                                    (t.assets_bulk_tags_label)
                                    input
                                        name="tags"
                                        placeholder="prod, web"
                                        list="asset-tags-list"
                                        disabled;
                                }
                                button type="submit" disabled { (t.assets_bulk_apply) }
                            }
                        }
                    }
                }
                datalist id="asset-tags-list" {
                    @for tag in all_tags {
                        option value=(tag) {}
                    }
                }
                dialog
                    class="asset-dialog"
                    id="add-asset-dialog"
                    aria-labelledby="add-asset-title"
                    onclick="if(event.target===this)this.close()"
                    onkeydown="if(event.key==='Escape'){event.preventDefault();this.close()}"
                    onclose="document.getElementById('open-add-asset').focus()" {
                    div.asset-drawer {
                        div.asset-drawer-header {
                            div {
                                h2 id="add-asset-title" { (t.assets_add_heading) }
                                p { (t.assets_add_intro) }
                            }
                            div.asset-drawer-header-actions {
                                span.status-chip { (t.draft_status) }
                                button
                                    class="drawer-close ghost-button"
                                    type="button"
                                    aria-label=(t.assets_close_drawer)
                                    onclick="this.closest('dialog').close()" {
                                    "×"
                                }
                            }
                        }
                        form method="post" action="/assets" {
                            (csrf_field(csrf_token))
                            input type="hidden" name="return_to" value=(return_to);
                            (asset_form_fields(t, None, credentials, true))
                            div.terminal-strip {
                                span { "$" }
                                span { "ssh -p 22 hop@target.internal" }
                            }
                            div.button-row.asset-drawer-actions {
                                button type="submit" { (t.save_asset) }
                                button
                                    class="ghost-button"
                                    type="button"
                                    onclick="this.closest('dialog').close()" {
                                    (t.assets_close_drawer)
                                }
                            }
                        }
                    }
                }
                dialog
                    class="asset-dialog"
                    id="edit-asset-dialog"
                    aria-labelledby="edit-asset-title"
                    onclick="if(event.target===this)this.close()"
                    onkeydown="if(event.key==='Escape'){event.preventDefault();this.close()}"
                    onclose="const target=document.getElementById(this.dataset.returnFocusId);if(target)target.focus()" {
                    div.asset-drawer {
                        div.asset-drawer-header {
                            div {
                                h2 id="edit-asset-title" { (t.edit_asset_title) }
                                p { (t.edit_asset_intro) }
                            }
                            div.asset-drawer-header-actions {
                                span.status-chip { (t.assets_edit_status) }
                                button
                                    class="drawer-close ghost-button"
                                    type="button"
                                    aria-label=(t.assets_close_drawer)
                                    onclick="this.closest('dialog').close()" {
                                    "×"
                                }
                            }
                        }
                        form method="post" action="/assets" data-edit-asset-form {
                            (csrf_field(csrf_token))
                            input type="hidden" name="return_to" value=(return_to);
                            (asset_form_fields(t, None, credentials, true))
                            div.button-row.asset-drawer-actions {
                                button type="submit" { (t.save_changes) }
                                button
                                    class="ghost-button"
                                    type="button"
                                    onclick="this.closest('dialog').close()" {
                                    (t.assets_close_drawer)
                                }
                            }
                        }
                    }
                }
                script {
                    (PreEscaped(r#"
                        (() => {
                            const form = document.querySelector('[data-assets-form]');
                            if (!form) return;
                            window.syncAssetBulkControls = () => {
                                const count = form.querySelectorAll('input[name="asset_ids"]:checked').length;
                                const controls = form.querySelector('[data-assets-bulk-controls]');
                                const countLabel = form.querySelector('[data-assets-selected-count]');
                                controls.hidden = count === 0;
                                controls.querySelectorAll('input, button').forEach((control) => {
                                    control.disabled = count === 0;
                                });
                                countLabel.textContent = `${count} ${countLabel.dataset.suffix}`;
                            };
                            window.addEventListener('pageshow', window.syncAssetBulkControls);
                            window.syncAssetBulkControls();

                            window.openAssetEditor = (trigger) => {
                                const dialog = document.getElementById('edit-asset-dialog');
                                const editForm = dialog.querySelector('[data-edit-asset-form]');
                                const fields = editForm.elements;
                                editForm.action = `/assets/${encodeURIComponent(trigger.dataset.assetId)}`;
                                fields.name.value = trigger.dataset.assetName;
                                fields.protocol.value = trigger.dataset.assetProtocol;
                                fields.hostname.value = trigger.dataset.assetHostname;
                                fields.port.value = trigger.dataset.assetPort;
                                fields.tags.value = trigger.dataset.assetTags;
                                fields.credential_id.value = trigger.dataset.assetCredential;
                                fields.description.value = trigger.dataset.assetDescription;
                                fields.return_to.value = trigger.dataset.returnTo;
                                const rdpHint = editForm.querySelector('[data-rdp-port-hint]');
                                if (rdpHint) rdpHint.hidden = fields.protocol.value !== 'rdp';
                                dialog.dataset.returnFocusId = trigger.id;
                                dialog.showModal();
                            };

                            window.copyAssetTarget = async (button) => {
                                const value = button.dataset.copyValue;
                                let copied = false;
                                try {
                                    await navigator.clipboard.writeText(value);
                                    copied = true;
                                } catch (_) {
                                    const fallback = document.createElement('textarea');
                                    fallback.value = value;
                                    fallback.setAttribute('readonly', '');
                                    fallback.style.position = 'fixed';
                                    fallback.style.opacity = '0';
                                    document.body.appendChild(fallback);
                                    fallback.select();
                                    copied = document.execCommand('copy');
                                    fallback.remove();
                                }

                                window.clearTimeout(button.copyResetTimer);
                                button.textContent = copied
                                    ? button.dataset.copySuccess
                                    : button.dataset.copyFailed;
                                button.classList.toggle('copy-success', copied);
                                button.copyResetTimer = window.setTimeout(() => {
                                    button.textContent = button.dataset.copyDefault;
                                    button.classList.remove('copy-success');
                                }, 1800);
                            };
                        })();
                    "#))
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
    ssh_port: u16,
    return_to: &str,
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
                    input type="hidden" name="return_to" value=(return_to);
                    (asset_form_fields(t, Some(asset), credentials, true))
                    datalist id="asset-tags-list" {
                        @for tag in all_tags {
                            option value=(tag) {}
                        }
                    }
                    div.button-row {
                        button type="submit" { (t.save_changes) }
                        a.ghost-button href=(return_to) { (t.back_to_assets) }
                    }
                }
            }
            @if let Some(command) = asset_tunnel_command(asset, ssh_port) {
                section.panel {
                    div.panel-header {
                        div {
                            h2 { (t.tunnel_command_heading) }
                            p { (t.tunnel_command_intro) }
                        }
                    }
                    div.command-block {
                        input class="command-input" readonly value=(command);
                    }
                }
            }
        },
    )
}

fn asset_form_fields(
    t: &L10n,
    asset: Option<&Asset>,
    credentials: &[Credential],
    autofocus: bool,
) -> Markup {
    let name = asset.map(|item| item.name.as_str()).unwrap_or("");
    let protocol = asset.map(asset_kind).unwrap_or(ASSET_PROTOCOL_SSH);
    let hostname = asset.map(|item| item.hostname.as_str()).unwrap_or("");
    let port = asset.map(|item| item.port).unwrap_or(22);
    let tags = asset.map(|item| item.tags.join(", ")).unwrap_or_default();
    let credential_id = asset.and_then(|item| item.credential_id.as_deref());
    let description = asset
        .and_then(|item| item.description.as_deref())
        .unwrap_or("");

    html! {
        div.grid {
            label.field {
                (t.field_name)
                input name="name" value=(name) required autofocus[autofocus];
            }
            label.field {
                (t.field_protocol)
                select name="protocol" onchange=(asset_protocol_onchange()) {
                    (asset_protocol_options(t, protocol))
                }
            }
            label.field {
                (t.field_hostname)
                input name="hostname" value=(hostname) required;
            }
            label.field {
                (t.field_port)
                input name="port" type="number" value=(port) required;
            }
            p class="fine-print field-wide" data-rdp-port-hint hidden[protocol != ASSET_PRESET_RDP] { (t.rdp_port_hint) }
            label.field {
                (t.field_tags)
                input name="tags" value=(tags) placeholder="prod, web" list="asset-tags-list";
            }
            label.field {
                (t.field_credential)
                select name="credential_id" {
                    option value="" selected[credential_id.is_none()] { (t.proxy_only) }
                    @for credential in credentials {
                        option value=(credential.id) selected[credential_id == Some(credential.id.as_str())] {
                            (credential.name) " (" (credential.username) ")"
                        }
                    }
                }
            }
            label.field.field-wide {
                (t.field_description)
                textarea name="description" { (description) }
            }
        }
    }
}

fn asset_protocol_options(t: &L10n, selected: &str) -> Markup {
    html! {
        option value=(ASSET_PROTOCOL_SSH) selected[selected == ASSET_PROTOCOL_SSH] { (t.protocol_ssh) }
        option value=(ASSET_PROTOCOL_TCP) selected[selected == ASSET_PROTOCOL_TCP] { (t.protocol_tcp) }
        option value=(ASSET_PRESET_RDP) selected[selected == ASSET_PRESET_RDP] { (t.protocol_rdp) }
        option value=(ASSET_PRESET_VNC) selected[selected == ASSET_PRESET_VNC] { (t.protocol_vnc) }
        option value=(ASSET_PRESET_MYSQL) selected[selected == ASSET_PRESET_MYSQL] { (t.protocol_mysql) }
        option value=(ASSET_PRESET_POSTGRES) selected[selected == ASSET_PRESET_POSTGRES] { (t.protocol_postgres) }
        option value=(ASSET_PRESET_REDIS) selected[selected == ASSET_PRESET_REDIS] { (t.protocol_redis) }
    }
}

fn assets_filter_href(tag: Option<&str>, search_query: Option<&str>) -> String {
    let mut parameters = Vec::new();
    if let Some(tag) = tag {
        parameters.push(format!("tag={}", url_query_value(tag)));
    }
    if let Some(query) = search_query {
        parameters.push(format!("q={}", url_query_value(query)));
    }

    if parameters.is_empty() {
        "/assets".to_string()
    } else {
        format!("/assets?{}", parameters.join("&"))
    }
}

fn asset_protocol_onchange() -> &'static str {
    "const p=this.form.querySelector('[name=port]'); const d={ssh:22,tcp:22,rdp:3389,vnc:5900,mysql:3306,postgres:5432,redis:6379}; if(p&&d[this.value])p.value=d[this.value]; const h=this.form.querySelector('[data-rdp-port-hint]'); if(h)h.hidden=this.value!=='rdp';"
}

fn asset_protocol_label<'a>(t: &'a L10n, protocol: &'a str) -> &'a str {
    match protocol {
        ASSET_PROTOCOL_SSH => t.protocol_ssh,
        ASSET_PROTOCOL_TCP => t.protocol_tcp,
        ASSET_PRESET_RDP => t.protocol_rdp,
        ASSET_PRESET_VNC => t.protocol_vnc,
        ASSET_PRESET_MYSQL => t.protocol_mysql,
        ASSET_PRESET_POSTGRES => t.protocol_postgres,
        ASSET_PRESET_REDIS => t.protocol_redis,
        other => other,
    }
}

fn asset_kind(asset: &Asset) -> &str {
    asset.preset.as_deref().unwrap_or(&asset.protocol)
}

fn asset_tunnel_command(asset: &Asset, ssh_port: u16) -> Option<String> {
    if asset.protocol == ASSET_PROTOCOL_SSH {
        return None;
    }
    let local_port = match asset.preset.as_deref() {
        Some(ASSET_PRESET_RDP) => 13389,
        Some(ASSET_PRESET_VNC) => 15900,
        Some(ASSET_PRESET_MYSQL) => 13306,
        Some(ASSET_PRESET_POSTGRES) => 15432,
        Some(ASSET_PRESET_REDIS) => 16379,
        _ => asset.port,
    };
    Some(format!(
        "ssh -p {ssh_port} -N -T -L 127.0.0.1:{local_port}:{}:{} hop-host",
        asset_tunnel_target(asset),
        asset.port
    ))
}

fn asset_tunnel_target(asset: &Asset) -> String {
    if asset
        .name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        format!("{}.hop", asset.name)
    } else {
        asset.hostname.clone()
    }
}

pub fn credentials(
    t: &L10n,
    items: &[Credential],
    assets: &[Asset],
    csrf_token: &str,
    error: Option<&str>,
    can_manage: bool,
) -> Markup {
    layout(
        t.credentials_title,
        "credentials",
        t,
        html! {
            div.credentials-page {
                div.console-hero {
                    div {
                        p { (t.credentials_intro) }
                    }
                    div.console-actions {
                        span.status-chip.good {
                            span.status-dot.good {}
                            (items.len()) " " (t.credentials_title)
                        }
                        @if can_manage {
                            button
                                id="open-add-credential"
                                type="button"
                                aria-haspopup="dialog"
                                aria-controls="add-credential-dialog"
                                onclick="document.getElementById('add-credential-dialog').showModal()" {
                                (t.credentials_add_heading)
                            }
                        }
                    }
                }
                @if let Some(error) = error {
                    p.error-message role="alert" { (error) }
                }
                section.panel.credentials-toolbar {
                    div {
                        strong { (t.credentials_export_heading) }
                        p.fine-print { (t.credentials_export_intro) }
                    }
                    div.button-row {
                        a.ghost-button href="/credentials/export?format=csv" { (t.export_csv) }
                        a.ghost-button href="/credentials/export?format=json" { (t.export_json) }
                        @if can_manage {
                            a.ghost-button href="/import" { (t.import_open) }
                        }
                    }
                }
                section.panel.credentials-inventory {
                    div.panel-header {
                        div {
                            h2 { (t.credentials_existing_heading) }
                            p { (t.credentials_existing_intro) }
                        }
                        span.status-chip { (items.len()) " " (t.credentials_title) }
                    }
                    div.table-wrap {
                        table.data-table {
                            thead {
                                tr {
                                    th { (t.field_name) }
                                    th { (t.field_username) }
                                    th { (t.field_auth_type) }
                                    th { (t.secrets_label) }
                                    th { (t.credential_usage_heading) }
                                    @if can_manage {
                                        th { (t.field_action) }
                                    }
                                }
                            }
                            tbody {
                                @if items.is_empty() {
                                    tr.empty-row {
                                        td colspan=(if can_manage { "6" } else { "5" }) {
                                            div.assets-empty-state {
                                                strong { (t.no_credentials) }
                                                @if can_manage {
                                                    button
                                                        type="button"
                                                        onclick="document.getElementById('open-add-credential').click()" {
                                                        (t.credentials_add_heading)
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                @for credential in items {
                                    @let used_assets = credential_assets(assets, &credential.id);
                                    @let usage_count = used_assets.len();
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
                                            @if usage_count == 0 {
                                                span.status-pill.neutral { (t.credential_unused) }
                                            } @else {
                                                div.primary-cell {
                                                    span.status-pill {
                                                        (usage_count) " " (t.credential_used_by_suffix)
                                                    }
                                                    span.subtle { (used_assets.join(", ")) }
                                                }
                                            }
                                        }
                                        @if can_manage {
                                            td {
                                                div.action-row {
                                                    a
                                                        id=(format!("edit-credential-{}", credential.id))
                                                        class="ghost-button"
                                                        href=(format!("/credentials/{}/edit", credential.id))
                                                        aria-haspopup="dialog"
                                                        aria-controls="edit-credential-dialog"
                                                        data-credential-id=(credential.id)
                                                        data-credential-name=(credential.name)
                                                        data-credential-username=(credential.username)
                                                        data-credential-auth-type=(credential.auth_type)
                                                        data-has-password=(credential.password_enc.is_some())
                                                        data-has-private-key=(credential.private_key_enc.is_some())
                                                        data-has-passphrase=(credential.passphrase_enc.is_some())
                                                        onclick="window.openCredentialEditor(this);return false" {
                                                        (t.edit)
                                                    }
                                                    form
                                                        method="post"
                                                        action=(format!("/credentials/{}/delete", credential.id))
                                                        data-confirm=(t.credential_delete_confirm)
                                                        onsubmit="return window.confirm(this.dataset.confirm)" {
                                                        (csrf_field(csrf_token))
                                                        button
                                                            class="danger"
                                                            type="submit"
                                                            disabled[usage_count > 0]
                                                            title=(if usage_count > 0 {
                                                                t.credential_delete_in_use
                                                            } else {
                                                                t.credential_delete_confirm
                                                            }) {
                                                            (t.delete)
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
                }
                dialog
                    class="admin-drawer-dialog"
                    id="add-credential-dialog"
                    aria-labelledby="add-credential-title"
                    onclick="if(event.target===this)this.close()"
                    onkeydown="if(event.key==='Escape'){event.preventDefault();this.close()}"
                    onclose="document.getElementById('open-add-credential').focus()" {
                    div.admin-drawer {
                        div.admin-drawer-header {
                            div {
                                h2 id="add-credential-title" { (t.credentials_add_heading) }
                                p { (t.credentials_add_intro) }
                            }
                            div.admin-drawer-header-actions {
                                span.status-chip { (t.credential_add_status) }
                                button
                                    class="drawer-close ghost-button"
                                    type="button"
                                    aria-label=(t.close)
                                    onclick="this.closest('dialog').close()" {
                                    "×"
                                }
                            }
                        }
                        form method="post" action="/credentials" data-credential-form data-mode="create" {
                            (csrf_field(csrf_token))
                            (credential_form_fields(t, None, false))
                            div.button-row.admin-drawer-actions {
                                button type="submit" { (t.save_credential) }
                                button
                                    class="ghost-button"
                                    type="button"
                                    onclick="this.closest('dialog').close()" {
                                    (t.close)
                                }
                            }
                        }
                    }
                }
                dialog
                    class="admin-drawer-dialog"
                    id="edit-credential-dialog"
                    aria-labelledby="edit-credential-title"
                    onclick="if(event.target===this)this.close()"
                    onkeydown="if(event.key==='Escape'){event.preventDefault();this.close()}"
                    onclose="const target=document.getElementById(this.dataset.returnFocusId);if(target)target.focus()" {
                    div.admin-drawer {
                        div.admin-drawer-header {
                            div {
                                h2 id="edit-credential-title" { (t.edit_credential_title) }
                                p { (t.edit_credential_intro) }
                            }
                            div.admin-drawer-header-actions {
                                span.status-chip { (t.credential_edit_status) }
                                button
                                    class="drawer-close ghost-button"
                                    type="button"
                                    aria-label=(t.close)
                                    onclick="this.closest('dialog').close()" {
                                    "×"
                                }
                            }
                        }
                        form
                            method="post"
                            action="/credentials"
                            data-credential-form
                            data-mode="edit"
                            data-secret-stored=(t.credential_secret_stored)
                            data-secret-missing=(t.credential_secret_missing) {
                            (csrf_field(csrf_token))
                            (credential_form_fields(t, None, true))
                            div.button-row.admin-drawer-actions {
                                button type="submit" { (t.save_changes) }
                                button
                                    class="ghost-button"
                                    type="button"
                                    onclick="this.closest('dialog').close()" {
                                    (t.close)
                                }
                            }
                        }
                    }
                }
                script { (PreEscaped(credential_drawer_script())) }
            }
        },
    )
}

pub fn edit_credential(
    t: &L10n,
    credential: &Credential,
    assets: &[Asset],
    csrf_token: &str,
) -> Markup {
    let used_assets = credential_assets(assets, &credential.id);
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
                    div
                        data-credential-form
                        data-mode="edit"
                        data-secret-stored=(t.credential_secret_stored)
                        data-secret-missing=(t.credential_secret_missing) {
                        (credential_form_fields(t, Some(credential), true))
                    }
                    div.button-row {
                        button type="submit" { (t.save_changes) }
                        a.ghost-button href="/credentials" { (t.back_to_credentials) }
                    }
                }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.credential_usage_heading) }
                        p {
                            @if used_assets.is_empty() {
                                (t.credential_unused)
                            } @else {
                                (used_assets.len()) " " (t.credential_used_by_suffix) ": "
                                (used_assets.join(", "))
                            }
                        }
                    }
                }
            }
            script { (PreEscaped(credential_drawer_script())) }
        },
    )
}

fn credential_assets(assets: &[Asset], credential_id: &str) -> Vec<String> {
    assets
        .iter()
        .filter(|asset| asset.credential_id.as_deref() == Some(credential_id))
        .map(|asset| asset.name.clone())
        .collect()
}

fn credential_form_fields(t: &L10n, credential: Option<&Credential>, editing: bool) -> Markup {
    let name = credential.map(|item| item.name.as_str()).unwrap_or("");
    let username = credential.map(|item| item.username.as_str()).unwrap_or("");
    let auth_type = credential
        .map(|item| item.auth_type.as_str())
        .unwrap_or("password");
    let has_password = credential.is_some_and(|item| item.password_enc.is_some());
    let has_private_key = credential.is_some_and(|item| item.private_key_enc.is_some());
    let has_passphrase = credential.is_some_and(|item| item.passphrase_enc.is_some());

    html! {
        div.grid {
            label.field {
                (t.field_name)
                input name="name" value=(name) required autofocus;
            }
            label.field {
                (t.field_username)
                input name="username" value=(username) autocomplete="off" required;
            }
            label.field.field-wide {
                (t.field_auth_type)
                select
                    name="auth_type"
                    data-credential-auth-type
                    onchange="window.hopToggleCredentialSecrets(this.closest('[data-credential-form]'))" {
                    option value="password" selected[auth_type == "password"] { "password" }
                    option value="key" selected[auth_type == "key"] { "key" }
                    option value="key+passphrase" selected[auth_type == "key+passphrase"] { "key+passphrase" }
                }
            }
            label
                class="field field-wide credential-secret-field"
                data-secret-for="password"
                hidden[auth_type != "password"] {
                (if editing { t.replace_password } else { t.field_password })
                input
                    type="password"
                    name="password"
                    autocomplete="new-password"
                    disabled[auth_type != "password"]
                    required[!editing && auth_type == "password"];
                @if editing {
                    span
                        class=(if has_password { "secret-state stored" } else { "secret-state" })
                        data-secret-state="password" {
                        (if has_password { t.credential_secret_stored } else { t.credential_secret_missing })
                    }
                }
            }
            label
                class="field field-wide credential-secret-field"
                data-secret-for="key key+passphrase"
                hidden[auth_type == "password"] {
                (if editing { t.replace_private_key } else { t.field_private_key })
                textarea
                    name="private_key"
                    rows="8"
                    autocomplete="off"
                    spellcheck="false"
                    disabled[auth_type == "password"]
                    required[!editing && auth_type != "password"] {}
                @if editing {
                    span
                        class=(if has_private_key { "secret-state stored" } else { "secret-state" })
                        data-secret-state="private-key" {
                        (if has_private_key { t.credential_secret_stored } else { t.credential_secret_missing })
                    }
                }
            }
            label
                class="field field-wide credential-secret-field"
                data-secret-for="key+passphrase"
                hidden[auth_type != "key+passphrase"] {
                (if editing { t.replace_passphrase } else { t.field_passphrase })
                input
                    type="password"
                    name="passphrase"
                    autocomplete="new-password"
                    disabled[auth_type != "key+passphrase"]
                    required[!editing && auth_type == "key+passphrase"];
                @if editing {
                    span
                        class=(if has_passphrase { "secret-state stored" } else { "secret-state" })
                        data-secret-state="passphrase" {
                        (if has_passphrase { t.credential_secret_stored } else { t.credential_secret_missing })
                    }
                }
            }
        }
        p.fine-print {
            (if editing { t.secret_keep_note } else { t.secret_storage_note })
        }
    }
}

fn credential_drawer_script() -> &'static str {
    r#"
        window.hopToggleCredentialSecrets = (form) => {
            if (!form) return;
            const type = form.querySelector('[data-credential-auth-type]').value;
            const creating = form.dataset.mode === 'create';
            form.querySelectorAll('[data-secret-for]').forEach((group) => {
                const visible = group.dataset.secretFor.split(' ').includes(type);
                group.hidden = !visible;
                group.querySelectorAll('input, textarea').forEach((field) => {
                    field.disabled = !visible;
                    field.required = creating && visible;
                });
            });
        };

        window.openCredentialEditor = (trigger) => {
            const dialog = document.getElementById('edit-credential-dialog');
            const form = dialog.querySelector('form');
            form.action = `/credentials/${encodeURIComponent(trigger.dataset.credentialId)}`;
            form.querySelector('[name="name"]').value = trigger.dataset.credentialName;
            form.querySelector('[name="username"]').value = trigger.dataset.credentialUsername;
            form.querySelector('[name="auth_type"]').value = trigger.dataset.credentialAuthType;
            form.querySelectorAll('[name="password"], [name="private_key"], [name="passphrase"]')
                .forEach((field) => field.value = '');

            const states = {
                password: trigger.dataset.hasPassword === 'true',
                'private-key': trigger.dataset.hasPrivateKey === 'true',
                passphrase: trigger.dataset.hasPassphrase === 'true'
            };
            form.querySelectorAll('[data-secret-state]').forEach((state) => {
                const stored = states[state.dataset.secretState];
                state.textContent = stored
                    ? form.dataset.secretStored
                    : form.dataset.secretMissing;
                state.classList.toggle('stored', stored);
            });

            window.hopToggleCredentialSecrets(form);
            dialog.dataset.returnFocusId = trigger.id;
            dialog.showModal();
        };

        document.querySelectorAll('[data-credential-form]').forEach(
            window.hopToggleCredentialSecrets
        );
    "#
}

pub fn keys(
    t: &L10n,
    items: &[AuthorizedKey],
    assets: &[Asset],
    csrf_token: &str,
    error: Option<&str>,
    can_manage: bool,
) -> Markup {
    layout(
        t.keys_title,
        "keys",
        t,
        html! {
            div.page-intro {
                h2 { (t.keys_heading) }
                p { (t.keys_intro) }
            }
            @if can_manage {
                section.panel {
                    div.panel-header {
                        div {
                            h2 { (t.keys_add_heading) }
                            p { (t.keys_add_intro) }
                        }
                    }
                    form method="post" action="/keys" {
                        (csrf_field(csrf_token))
                        @if let Some(error) = error {
                            p.error-message role="alert" { (error) }
                        }
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
                        (key_access_selector(t, assets, false, &[]))
                        div.button-row {
                            button type="submit" { (t.save_key) }
                        }
                    }
                    script { (PreEscaped(key_access_script())) }
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
                            tr {
                                th { (t.field_name) }
                                th { (t.field_fingerprint) }
                                th { (t.key_access_mode) }
                                th { (t.field_status) }
                                @if can_manage {
                                    th { (t.field_action) }
                                }
                            }
                        }
                        tbody {
                            @if items.is_empty() {
                                tr.empty-row {
                                    td colspan=(if can_manage { "5" } else { "4" }) { (t.no_keys) }
                                }
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
                                        span.status-pill.neutral {
                                            (if key.asset_access_mode == AssetAccessMode::Restricted {
                                                t.key_access_restricted
                                            } else {
                                                t.key_access_all
                                            })
                                        }
                                    }
                                    td {
                                        @if key.is_active {
                                            span.status-pill { (t.active) }
                                        } @else {
                                            span.status-pill.neutral { (t.inactive) }
                                        }
                                    }
                                    @if can_manage {
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
            }
        },
    )
}

pub fn edit_key(
    t: &L10n,
    key: &AuthorizedKey,
    assets: &[Asset],
    assigned_ids: &[String],
    csrf_token: &str,
    error: Option<&str>,
) -> Markup {
    let restricted = key.asset_access_mode == AssetAccessMode::Restricted;
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
                    @if let Some(error) = error {
                        p.error-message { (error) }
                    }
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
                    (key_access_selector(t, assets, restricted, assigned_ids))
                    div.button-row {
                        button type="submit" { (t.save_changes) }
                        a.ghost-button href="/keys" { (t.back_to_keys) }
                    }
                }
            }
            script { (PreEscaped(key_access_script())) }
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

fn key_access_selector(
    t: &L10n,
    assets: &[Asset],
    restricted: bool,
    assigned_ids: &[String],
) -> Markup {
    let accessible_count = if restricted {
        assigned_ids.len()
    } else {
        assets.len()
    };
    html! {
        section.asset-access-list {
            div.panel-header {
                div {
                    h2 { (t.key_access_heading) }
                    p { (t.key_access_intro) }
                }
                span.status-pill {
                    (accessible_count) " / " (assets.len()) " " (t.key_assets_suffix)
                }
            }
            label.field {
                (t.key_access_mode)
                select name="asset_access_mode" data-asset-access-mode onchange=(key_access_mode_onchange()) {
                    option value="all" selected[!restricted] { (t.key_access_all) }
                    option value="restricted" selected[restricted] { (t.key_access_restricted) }
                }
            }
            p.fine-print data-access-all-note hidden[restricted] { (t.key_access_all_intro) }
            div data-asset-access-list hidden[!restricted] {
                p.fine-print { (t.key_access_restricted_intro) }
                label.field {
                    (t.key_asset_search)
                    input type="search" data-asset-filter oninput=(key_asset_filter_oninput());
                }
                div.asset-access-list {
                    @for asset in assets {
                        @let assigned = assigned_ids.iter().any(|id| id == &asset.id);
                        @let search = format!(
                            "{} {} {} {} {}",
                            asset.name,
                            asset_kind(asset),
                            asset.hostname,
                            asset.port,
                            asset.tags.join(" ")
                        ).to_ascii_lowercase();
                        label.asset-access-item data-asset-search=(search) {
                            input type="checkbox" name="asset_id" value=(asset.id)
                                checked[assigned] disabled[!restricted];
                            div.primary-cell {
                                span { (asset.name) }
                                span.subtle {
                                    (asset_protocol_label(t, asset_kind(asset))) " · "
                                    (asset.hostname) ":" (asset.port)
                                }
                                @if !asset.tags.is_empty() {
                                    div.tag-list {
                                        @for tag in &asset.tags {
                                            span.tag { (tag) }
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
}

fn key_access_mode_onchange() -> &'static str {
    "window.hopToggleKeyAccess(this.form)"
}

fn key_asset_filter_oninput() -> &'static str {
    "const q=this.value.toLowerCase();this.form.querySelectorAll('[data-asset-search]').forEach((row)=>row.hidden=!row.dataset.assetSearch.includes(q))"
}

fn key_access_script() -> &'static str {
    r#"window.hopToggleKeyAccess=function(form){const mode=form.querySelector('[data-asset-access-mode]').value;const restricted=mode==='restricted';const list=form.querySelector('[data-asset-access-list]');const note=form.querySelector('[data-access-all-note]');list.hidden=!restricted;note.hidden=restricted;list.querySelectorAll('input[type=checkbox]').forEach((input)=>input.disabled=!restricted);};"#
}

pub fn known_hosts(
    t: &L10n,
    items: &[KnownHost],
    assets: &[Asset],
    csrf_token: &str,
    can_manage: bool,
) -> Markup {
    layout(
        t.known_hosts_title,
        "known-hosts",
        t,
        html! {
            div.known-hosts-page {
                div.console-hero {
                    div {
                        p { (t.known_hosts_intro) }
                    }
                    div.console-actions {
                        span.status-chip.good {
                            span.status-dot.good {}
                            (items.len()) " " (t.known_hosts_trusted)
                        }
                    }
                }
                section.panel.trust-reset-note {
                    div {
                        strong { (t.known_hosts_trusted) }
                        p { (t.known_hosts_panel_intro) }
                    }
                    p.fine-print { (t.known_hosts_reset_note) }
                }
                section.panel.known-hosts-inventory {
                    div.panel-header {
                        div {
                            h2 { (t.known_hosts_panel_heading) }
                            p { (t.known_hosts_panel_intro) }
                        }
                    }
                    div.table-wrap {
                        table.data-table {
                            thead {
                                tr {
                                    th { (t.host_column) }
                                    th { (t.key_type_column) }
                                    th { (t.field_fingerprint) }
                                    th { (t.first_seen_column) }
                                    th { (t.known_hosts_asset_usage) }
                                    @if can_manage {
                                        th { (t.field_action) }
                                    }
                                }
                            }
                            tbody {
                                @if items.is_empty() {
                                    tr.empty-row {
                                        td colspan=(if can_manage { "6" } else { "5" }) { (t.no_known_hosts) }
                                    }
                                }
                                @for host in items {
                                    @let used_assets = known_host_assets(assets, host);
                                    tr {
                                        td {
                                            div.primary-cell {
                                                (host.hostname)
                                                span.subtle.mono { ":" (host.port) }
                                            }
                                        }
                                        td { span.status-pill.neutral { (host.key_type) } }
                                        td {
                                            div.fingerprint-copy-group {
                                                code.fingerprint-value { (host.fingerprint) }
                                                button
                                                    class="target-copy-button ghost-button"
                                                    type="button"
                                                    data-copy-value=(host.fingerprint)
                                                    data-copy-default=(t.known_host_copy_fingerprint)
                                                    data-copy-success=(t.asset_copy_success)
                                                    data-copy-failed=(t.asset_copy_failed)
                                                    aria-label=(format!(
                                                        "{}: {}",
                                                        t.known_host_copy_fingerprint,
                                                        host.fingerprint
                                                    ))
                                                    aria-live="polite"
                                                    onclick="window.copyKnownHostFingerprint(this)" {
                                                    (t.known_host_copy_fingerprint)
                                                }
                                            }
                                        }
                                        td {
                                            div.primary-cell {
                                                span.status-pill { (t.known_hosts_trusted) }
                                                span.subtle { (host.first_seen.as_deref().unwrap_or("-")) }
                                            }
                                        }
                                        td {
                                            @if used_assets.is_empty() {
                                                span.status-pill.neutral { (t.known_hosts_unused) }
                                            } @else {
                                                div.primary-cell {
                                                    span.status-pill {
                                                        (used_assets.len()) " " (t.known_hosts_assets_suffix)
                                                    }
                                                    span.subtle { (used_assets.join(", ")) }
                                                }
                                            }
                                        }
                                        @if can_manage {
                                            td {
                                                button
                                                    id=(format!(
                                                        "reset-known-host-{}-{}-{}",
                                                        host.hostname,
                                                        host.port,
                                                        host.key_type
                                                    ))
                                                    class="danger"
                                                    type="button"
                                                    aria-haspopup="dialog"
                                                    aria-controls="reset-known-host-dialog"
                                                    data-hostname=(host.hostname)
                                                    data-port=(host.port)
                                                    data-key-type=(host.key_type)
                                                    data-fingerprint=(host.fingerprint)
                                                    onclick="window.openKnownHostReset(this)" {
                                                    (t.known_host_reset_action)
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                dialog
                    class="confirm-dialog"
                    id="reset-known-host-dialog"
                    aria-labelledby="reset-known-host-title"
                    onclick="if(event.target===this)this.close()"
                    onkeydown="if(event.key==='Escape'){event.preventDefault();this.close()}"
                    onclose="const target=document.getElementById(this.dataset.returnFocusId);if(target)target.focus()" {
                    div.confirm-dialog-card {
                        div.confirm-dialog-header {
                            div {
                                p.eyebrow { (t.known_host_reset_action) }
                                h2 id="reset-known-host-title" { (t.known_host_reset_heading) }
                            }
                            button
                                class="drawer-close ghost-button"
                                type="button"
                                aria-label=(t.close)
                                onclick="this.closest('dialog').close()" {
                                "×"
                            }
                        }
                        p { (t.known_host_reset_intro) }
                        div.trust-reset-target {
                            strong data-reset-host {}
                            code data-reset-fingerprint {}
                        }
                        p.warning-message { (t.known_host_reset_warning) }
                        form method="post" action="/known-hosts" data-known-host-reset-form {
                            (csrf_field(csrf_token))
                            input type="hidden" name="key_type";
                            label.confirm-check {
                                input
                                    type="checkbox"
                                    name="confirm_reset"
                                    value="yes"
                                    onchange="this.form.querySelector('[data-reset-submit]').disabled=!this.checked";
                                span { (t.known_host_reset_confirm) }
                            }
                            div.button-row {
                                button
                                    class="danger"
                                    type="submit"
                                    data-reset-submit
                                    disabled {
                                    (t.known_host_reset_submit)
                                }
                                button
                                    class="ghost-button"
                                    type="button"
                                    onclick="this.closest('dialog').close()" {
                                    (t.close)
                                }
                            }
                        }
                    }
                }
                script { (PreEscaped(known_host_script())) }
            }
        },
    )
}

fn known_host_assets(assets: &[Asset], host: &KnownHost) -> Vec<String> {
    assets
        .iter()
        .filter(|asset| asset.hostname == host.hostname && asset.port == host.port)
        .map(|asset| asset.name.clone())
        .collect()
}

fn known_host_script() -> &'static str {
    r#"
        window.copyKnownHostFingerprint = async (button) => {
            const value = button.dataset.copyValue;
            let copied = false;
            try {
                await navigator.clipboard.writeText(value);
                copied = true;
            } catch (_) {
                const fallback = document.createElement('textarea');
                fallback.value = value;
                fallback.setAttribute('readonly', '');
                fallback.style.position = 'fixed';
                fallback.style.opacity = '0';
                document.body.appendChild(fallback);
                fallback.select();
                copied = document.execCommand('copy');
                fallback.remove();
            }

            window.clearTimeout(button.copyResetTimer);
            button.textContent = copied
                ? button.dataset.copySuccess
                : button.dataset.copyFailed;
            button.classList.toggle('copy-success', copied);
            button.copyResetTimer = window.setTimeout(() => {
                button.textContent = button.dataset.copyDefault;
                button.classList.remove('copy-success');
            }, 1800);
        };

        window.openKnownHostReset = (trigger) => {
            const dialog = document.getElementById('reset-known-host-dialog');
            const form = dialog.querySelector('[data-known-host-reset-form]');
            form.action = `/known-hosts/${encodeURIComponent(trigger.dataset.hostname)}/${trigger.dataset.port}/delete`;
            form.querySelector('[name="key_type"]').value = trigger.dataset.keyType;
            form.querySelector('[name="confirm_reset"]').checked = false;
            form.querySelector('[data-reset-submit]').disabled = true;
            dialog.querySelector('[data-reset-host]').textContent =
                `${trigger.dataset.hostname}:${trigger.dataset.port} · ${trigger.dataset.keyType}`;
            dialog.querySelector('[data-reset-fingerprint]').textContent =
                trigger.dataset.fingerprint;
            dialog.dataset.returnFocusId = trigger.id;
            dialog.showModal();
        };
    "#
}

pub fn sessions(
    t: &L10n,
    items: &[Session],
    admin_events: &[AuditEvent],
    active_session_ids: &[String],
    csrf_token: &str,
    can_terminate: bool,
) -> Markup {
    layout(
        t.sessions_title,
        "sessions",
        t,
        html! {
            div.audit-page {
                div.console-hero {
                    div {
                        h2 { (t.sessions_heading) }
                        p { (t.sessions_intro) }
                    }
                    div.console-actions {
                        span.status-chip.good { (active_session_ids.len()) " " (t.sessions_active_heading) }
                        span.status-chip.danger { (items.iter().filter(|session| session.status == "failed").count()) " " (t.sessions_failed_suffix) }
                        span.status-chip.neutral { (admin_events.len()) " " (t.sessions_admin_recorded_suffix) }
                        span.status-chip.good { (items.len()) " " (t.sessions_recorded_suffix) }
                        @if can_terminate && !active_session_ids.is_empty() {
                            form method="post" action="/sessions/terminate-all" {
                                (csrf_field(csrf_token))
                                button.danger type="submit" { (t.sessions_terminate_all) }
                            }
                        }
                        a.ghost-button href="/sessions" { (t.sessions_live_tail) }
                    }
                }
                div.audit-toolbar {
                    div.terminal-strip {
                        span { "hop admin sessions --limit 100" }
                    }
                    div.status-row {
                        span.command-chip { (t.sessions_range_latest) }
                        span.command-chip { (t.sessions_user_all) }
                        span.command-chip { (t.sessions_event_all) }
                    }
                }
                section.panel {
                    div.panel-header {
                        div {
                            h2 { (t.sessions_admin_heading) }
                            p { (t.sessions_admin_intro) }
                        }
                    }
                    div.table-wrap {
                        table.data-table {
                            thead {
                                tr {
                                    th { (t.started_column) }
                                    th { (t.sessions_actor_column) }
                                    th { (t.field_action) }
                                    th { (t.target_column) }
                                    th { (t.error_column) }
                                    th { (t.field_status) }
                                }
                            }
                            tbody {
                                @if admin_events.is_empty() {
                                    tr.empty-row { td colspan="6" { (t.sessions_no_admin_events) } }
                                }
                                @for event in admin_events {
                                    tr {
                                        td.mono { (event.occurred_at.as_deref().unwrap_or("-")) }
                                        td {
                                            div.primary-cell {
                                                (event.actor_label)
                                                @if let Some(source_ip) = &event.source_ip {
                                                    span.subtle { (t.sessions_source_prefix) " " (source_ip) }
                                                }
                                            }
                                        }
                                        td { span.audit-event { (event.action) } }
                                        td {
                                            div.primary-cell {
                                                (event.target_label.as_deref().unwrap_or(event.target_type.as_str()))
                                                @if let Some(target_id) = &event.target_id {
                                                    span.subtle.mono { (target_id) }
                                                }
                                            }
                                        }
                                        td {
                                            span.subtle.mono { (event.details_json.as_deref().unwrap_or("-")) }
                                        }
                                        td {
                                            @if event.result == "failure" {
                                                span.status-pill.danger { (event.result) }
                                            } @else {
                                                span.status-pill { (event.result) }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                div.audit-grid {
                    section.panel {
                            div.panel-header {
                                div {
                                    h2 { (t.sessions_recent_heading) }
                                    p { (t.sessions_recent_intro) " " (t.sessions_active_intro) }
                                }
                            }
                        div.table-wrap {
                            table.data-table {
                                thead {
                                    tr {
                                        th { (t.started_column) }
                                        th { (t.key_column) }
                                        th { (t.asset_column) }
                                        th { (t.mode_column) }
                                        th { (t.error_column) }
                                        th { (t.field_status) }
                                        th { (t.sessions_action_column) }
                                    }
                                }
                                tbody {
                                    @if items.is_empty() {
                                        tr.empty-row { td colspan="7" { (t.no_sessions) } }
                                    }
                                    @for session in items {
                                        @let is_active = active_session_ids.iter().any(|id| id == &session.id);
                                        tr {
                                            td.mono { (session.started_at.as_deref().unwrap_or("-")) }
                                            td {
                                                div.primary-cell {
                                                    (session.key_name.as_deref().unwrap_or("-"))
                                                    span.subtle.mono { (session.key_finger) }
                                                }
                                            }
                                            td { (session.asset_name.as_deref().unwrap_or("-")) }
                                            td { span.audit-event { (session_event_label(session)) } }
                                            td {
                                                div.primary-cell {
                                                    span.mono {
                                                        @if let Some(target_host) = &session.target_host {
                                                            (target_host) ":" (session.target_port.unwrap_or_default())
                                                        } @else {
                                                            "-"
                                                        }
                                                    }
                                                    @if let Some(client_ip) = &session.client_ip {
                                                        span.subtle { (t.sessions_source_prefix) " " (client_ip) }
                                                    }
                                                    @if let Some(error) = &session.error {
                                                        span.subtle { (error) }
                                                    }
                                                }
                                            }
                                            td {
                                                @if is_active {
                                                    span.status-pill { (t.sessions_active_heading) }
                                                } @else if session.status == "failed" {
                                                    span.status-pill.danger { (session.status) }
                                                } @else if session.status == "ok" {
                                                    span.status-pill { (session.status) }
                                                } @else {
                                                    span.status-pill.neutral { (session.status) }
                                                }
                                            }
                                            td.table-action {
                                                @if can_terminate && is_active {
                                                    form method="post" action=(format!("/sessions/{}/terminate", session.id)) {
                                                        (csrf_field(csrf_token))
                                                        button.danger type="submit" { (t.sessions_terminate) }
                                                    }
                                                } @else {
                                                    span.subtle { "-" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    div.panel-stack {
                        section.panel {
                            div.panel-header {
                                div {
                                    h2 { (t.sessions_summary_heading) }
                                    p { (t.sessions_summary_intro) }
                                }
                            }
                            div.incident-list {
                                div.incident-item {
                                    span.status-dot.danger {}
                                    b { (t.sessions_failed_heading) }
                                    span { (items.iter().filter(|session| session.status == "failed").count()) }
                                }
                                div.incident-item {
                                    span.status-dot.warn {}
                                    b { (t.sessions_direct_heading) }
                                    span { (items.iter().filter(|session| session.mode == "direct").count()) }
                                }
                                div.incident-item {
                                    span.status-dot.good {}
                                    b { (t.sessions_completed_heading) }
                                    span { (items.iter().filter(|session| session.ended_at.is_some()).count()) }
                                }
                            }
                        }
                    }
                }
            }
        },
    )
}

fn session_event_label(session: &Session) -> &str {
    session.mode.as_str()
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

fn nav_link(href: &str, label: &str, icon: &str, active: bool) -> Markup {
    if active {
        html! {
            a class="nav-link active" href=(href) aria-current="page" {
                (PreEscaped(icon))
                (label)
            }
        }
    } else {
        html! {
            a class="nav-link" href=(href) {
                (PreEscaped(icon))
                (label)
            }
        }
    }
}

fn mobile_nav_link(href: &str, label: &str, icon: &str, active: bool) -> Markup {
    if active {
        html! {
            a class="mobile-tab active" href=(href) aria-current="page" {
                (PreEscaped(icon))
                span { (label) }
            }
        }
    } else {
        html! {
            a class="mobile-tab" href=(href) {
                (PreEscaped(icon))
                span { (label) }
            }
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
        "settings" => "/settings",
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
    use super::super::i18n::{EN, ZH};
    use super::*;

    #[test]
    fn mutating_forms_include_csrf_token() {
        let rendered = assets(&EN, &[], &[], "csrf-123", None, None, &[], 2222, true).into_string();

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
        assert!(rendered.contains(r#"href="/settings""#));
        assert!(rendered.contains("/set-language?lang=zh"));
    }

    #[test]
    fn layout_uses_operator_console_theme_tokens() {
        let rendered =
            layout(EN.assets_title, "assets", &EN, html! { p { "content" } }).into_string();

        assert!(rendered.contains(r#"data-theme="operator""#));
        assert!(rendered.contains("--canvas: #0d1117"));
        assert!(rendered.contains("--sidebar: #0a0f16"));
        assert!(rendered.contains("--control: #3b82f6"));
        assert!(rendered.contains("--console-green: #22c55e"));
        assert!(rendered.contains("font-family: Inter, system-ui"));
        assert!(rendered.contains(".mobile-tabbar"));
        assert!(rendered.contains("min-width: 0"));
        assert!(rendered.contains("max-width: 100%"));
        assert!(rendered.contains("@media (prefers-reduced-motion: reduce)"));
    }

    #[test]
    fn layout_css_does_not_emit_html_escaped_quotes() {
        let rendered =
            layout(EN.assets_title, "assets", &EN, html! { p { "content" } }).into_string();

        assert!(!rendered.contains("&quot;"));
    }

    #[test]
    fn layout_localizes_mobile_navigation_labels() {
        let rendered =
            layout(ZH.assets_title, "assets", &ZH, html! { p { "content" } }).into_string();

        assert!(rendered.contains("概览"));
        assert!(rendered.contains("资产"));
        assert!(rendered.contains("审计日志"));
        assert!(rendered.contains("设置"));
        assert!(!rendered.contains(">Dash<"));
        assert!(!rendered.contains(">Assets<"));
        assert!(!rendered.contains(">Audit<"));
        assert!(!rendered.contains(">Admin<"));
    }

    #[test]
    fn login_uses_a_dedicated_shell_without_internal_navigation() {
        let rendered = login(&EN, None, false, None).into_string();

        assert!(rendered.contains(r#"class="admin-shell login-shell""#));
        assert!(rendered.contains(r#"class="login-brand""#));
        assert!(rendered.contains("Loopback admin"));
        assert!(rendered.contains("/set-language?lang=zh&amp;redirect=%2Flogin"));
        assert!(!rendered.contains(r#"class="sidebar""#));
        assert!(!rendered.contains(r#"class="mobile-tabbar""#));
        assert!(!rendered.contains(r#"href="/""#));
        assert!(!rendered.contains(r#"href="/assets""#));
        assert!(!rendered.contains(r#"href="/credentials""#));
        assert!(!rendered.contains(r#"href="/keys""#));
        assert!(!rendered.contains(r#"href="/known-hosts""#));
        assert!(!rendered.contains(r#"href="/sessions""#));
        assert!(!rendered.contains(r#"href="/import""#));
        assert!(!rendered.contains(r#"href="/settings""#));
    }

    #[test]
    fn login_password_field_supports_password_managers_and_accessible_errors() {
        let rendered = login(&EN, Some("Invalid password"), false, None).into_string();

        assert!(rendered.contains(r#"id="login-password""#));
        assert!(rendered.contains(r#"autocomplete="current-password""#));
        assert!(rendered.contains("autofocus"));
        assert!(rendered.contains(r#"id="login-error" role="alert""#));
        assert!(rendered.contains(r#"aria-describedby="login-error""#));
        assert!(rendered.contains(r#"aria-invalid="true""#));

        let without_error = login(&EN, None, false, None).into_string();
        assert!(!without_error.contains(r#"id="login-error""#));
        assert!(!without_error.contains(r#"aria-describedby="login-error""#));
        assert_eq!(without_error.matches(r#"aria-invalid="true""#).count(), 1);
        assert_eq!(rendered.matches(r#"aria-invalid="true""#).count(), 2);
    }

    #[test]
    fn login_adds_account_name_only_after_team_mode_is_enabled() {
        let single = login(&EN, None, false, None).into_string();
        let team = login(&EN, None, true, Some("alice")).into_string();

        assert!(!single.contains(r#"name="username""#));
        assert!(team.contains(r#"name="username""#));
        assert!(team.contains(r#"value="alice""#));
        assert!(team.contains(r#"autocomplete="username""#));
        assert!(team.contains(EN.login_team_intro));
    }

    #[test]
    fn overview_renders_metric_tiles_with_labels() {
        let rendered = overview(&EN, &dashboard_data()).into_string();

        assert!(rendered.contains(r#"class="dashboard-page""#));
        assert!(rendered.contains(r#"class="metric-grid""#));
        assert!(rendered.contains(r#"class="metric-value""#));
        assert!(rendered.contains("Bastion posture"));
        assert!(rendered.contains("Total servers"));
        assert!(rendered.contains("Sessions · 24h"));
        assert!(rendered.contains("Managed SSH coverage"));
        assert!(rendered.contains("Gateway posture"));
        assert!(rendered.contains("Recent SSH access"));
        assert!(rendered.contains("Recent admin changes"));
        assert!(rendered.contains("asset.update"));
        assert!(rendered.contains("50%"));
        assert!(rendered.contains("1 / 2"));
        assert!(!rendered.contains("Admin Web available"));
        assert!(!rendered.contains("Live Sessions"));
        assert!(!rendered.contains("Activity Heatmap"));
        assert!(!rendered.contains("JIT approvals pending"));
        assert!(!rendered.contains("Failed login watch"));
    }

    #[test]
    fn dashboard_helpers_keep_zero_denominators_and_durations_explainable() {
        assert_eq!(coverage_percent(0, 0), 0);
        assert_eq!(coverage_percent(1, 3), 33);
        assert_eq!(uptime_label(3_661), "1h 1m");
        let session = Session {
            id: "session-duration".to_string(),
            key_finger: "SHA256:test".to_string(),
            key_name: Some("alice".to_string()),
            mode: "direct".to_string(),
            asset_name: Some("prod-api".to_string()),
            target_host: Some("10.0.0.8".to_string()),
            target_port: Some(22),
            client_ip: None,
            status: "ok".to_string(),
            error: None,
            started_at: Some("2026-07-28 10:00:00".to_string()),
            ended_at: Some("2026-07-28 10:02:05".to_string()),
        };
        assert_eq!(session_duration_label(&session), "2m 5s");
    }

    #[test]
    fn assets_page_renders_tag_filters_and_bulk_editor() {
        let tags = vec!["prod".to_string(), "web".to_string()];
        let rendered = assets(
            &EN,
            &[],
            &[],
            "csrf-123",
            Some("prod"),
            Some("api east"),
            &tags,
            2222,
            true,
        )
        .into_string();

        assert!(rendered.contains(r#"class="assets-page""#));
        assert!(rendered.contains("Inventory, connectivity, and assigned access tags."));
        assert!(rendered.contains("Server inventory"));
        assert!(rendered.contains("Add Asset"));
        assert!(!rendered.contains("<h2>Assets / Servers</h2>"));
        assert!(rendered.contains(r#"method="get" action="/assets""#));
        assert!(rendered
            .contains(r#"<label class="field" for="asset-search-query">Search assets</label>"#));
        assert!(rendered.contains(r#"type="search" name="q" value="api east""#));
        assert!(rendered.contains(">Search</button>"));
        assert!(rendered.contains(">Clear filters</a>"));
        assert!(rendered.contains(r#"type="hidden" name="tag" value="prod""#));
        assert!(rendered.contains(r#"href="/assets?q=api%20east""#));
        assert!(rendered.contains(r#"href="/assets?tag=prod&amp;q=api%20east""#));
        assert!(rendered.contains(r#"action="/assets/bulk-tags""#));
        assert!(rendered.contains(r#"data-assets-bulk-controls hidden"#));
        assert!(rendered.contains(r#"list="asset-tags-list""#));
    }

    #[test]
    fn assets_page_uses_an_accessible_on_demand_drawer() {
        let rendered = assets(&EN, &[], &[], "csrf-123", None, None, &[], 2222, true).into_string();

        assert!(rendered.contains(r#"aria-haspopup="dialog""#));
        assert!(rendered.contains(r#"aria-controls="add-asset-dialog""#));
        assert!(rendered.contains(r#"<dialog class="asset-dialog" id="add-asset-dialog""#));
        assert!(rendered.contains(r#"aria-labelledby="add-asset-title""#));
        assert!(rendered.contains("showModal()"));
        assert!(rendered.contains("event.target===this"));
        assert!(rendered.contains("event.key==='Escape'"));
        assert!(rendered.contains(r#"onclose="document.getElementById('open-add-asset').focus()""#));
        assert!(rendered.contains(r#"aria-label="Close""#));
        assert!(!rendered.contains(r#"class="panel asset-form-panel""#));
    }

    #[test]
    fn assets_page_keeps_targets_intact_and_bulk_controls_progressive() {
        let item = asset("api", "api.internal.example", &["prod"]);
        let rendered =
            assets(&EN, &[item], &[], "csrf-123", None, None, &[], 2222, true).into_string();

        assert!(rendered.contains(r#"class="target-address""#));
        assert!(rendered.contains("api.internal.example:22"));
        assert!(rendered.contains(r#"class="target-copy-button ghost-button""#));
        assert!(rendered.contains(r#"data-copy-value="api.internal.example:22""#));
        assert!(rendered.contains(r#"aria-label="Copy target: api.internal.example:22""#));
        assert!(rendered.contains("window.copyAssetTarget"));
        assert!(rendered.contains(r#"data-assets-bulk-controls hidden"#));
        assert!(rendered
            .contains(r#"name="tags" placeholder="prod, web" list="asset-tags-list" disabled"#));
        assert!(rendered.contains(r#"type="submit" disabled"#));
        assert!(rendered.contains("syncAssetBulkControls"));
    }

    #[test]
    fn assets_page_opens_edit_drawer_with_current_asset_and_filter_state() {
        let mut item = asset("api", "api.internal.example", &["prod", "web"]);
        item.description = Some("Primary API".to_string());
        let rendered = assets(
            &EN,
            &[item],
            &[],
            "csrf-123",
            Some("prod"),
            Some("api"),
            &[],
            2222,
            true,
        )
        .into_string();

        assert!(rendered.contains(r#"aria-controls="edit-asset-dialog""#));
        assert!(rendered.contains(r#"data-asset-name="api""#));
        assert!(rendered.contains(r#"data-asset-hostname="api.internal.example""#));
        assert!(rendered.contains(r#"data-asset-tags="prod, web""#));
        assert!(rendered.contains(r#"data-asset-description="Primary API""#));
        assert!(rendered
            .contains(r#"href="/assets/api/edit?return_to=%2Fassets%3Ftag%3Dprod%26q%3Dapi""#));
        assert!(rendered.contains(r#"<dialog class="asset-dialog" id="edit-asset-dialog""#));
        assert!(rendered.contains(r#"data-edit-asset-form"#));
        assert!(rendered.contains(r#"name="return_to" value="/assets?tag=prod&amp;q=api""#));
        assert!(rendered.contains("window.openAssetEditor"));
        assert!(rendered.contains("returnFocusId"));
    }

    #[test]
    fn standalone_asset_edit_keeps_return_location_and_reuses_form_fields() {
        let item = asset("api", "api.internal.example", &["prod"]);
        let rendered = edit_asset(
            &EN,
            &item,
            &[],
            "csrf-123",
            &["prod".to_string()],
            2222,
            "/assets?tag=prod",
        )
        .into_string();

        assert!(rendered.contains(r#"name="return_to" value="/assets?tag=prod""#));
        assert!(rendered.contains(r#"href="/assets?tag=prod""#));
        assert!(rendered.contains(r#"name="name" value="api""#));
        assert!(rendered.contains(r#"name="hostname" value="api.internal.example""#));
    }

    #[test]
    fn assets_page_distinguishes_inventory_and_filter_empty_states() {
        let no_assets =
            assets(&EN, &[], &[], "csrf-123", None, None, &[], 2222, true).into_string();
        let no_matches = assets(
            &EN,
            &[],
            &[],
            "csrf-123",
            Some("prod"),
            Some("missing"),
            &[],
            2222,
            true,
        )
        .into_string();

        assert!(no_assets.contains(EN.no_assets));
        assert!(!no_matches.contains(EN.no_assets));
        assert!(no_matches.contains(EN.assets_filter_heading));
        assert!(no_matches.contains(EN.assets_filter_intro));
    }

    #[test]
    fn credentials_page_uses_drawers_secret_states_and_usage_guards() {
        let credential = credential("cred-1", "prod-root", "root", "password");
        let mut assigned_asset = asset("api", "api.internal", &["prod"]);
        assigned_asset.credential_id = Some("cred-1".to_string());
        let rendered = credentials(
            &EN,
            &[credential],
            &[assigned_asset],
            "csrf-123",
            None,
            true,
        )
        .into_string();

        assert!(rendered.contains(r#"aria-controls="add-credential-dialog""#));
        assert!(rendered.contains(r#"aria-controls="edit-credential-dialog""#));
        assert!(
            rendered.contains(r#"<dialog class="admin-drawer-dialog" id="add-credential-dialog""#)
        );
        assert!(
            rendered.contains(r#"<dialog class="admin-drawer-dialog" id="edit-credential-dialog""#)
        );
        assert!(rendered.contains(r#"autocomplete="new-password""#));
        assert!(rendered.contains(r#"data-secret-for="key key+passphrase" hidden"#));
        assert!(rendered.contains(r#"data-has-password="true""#));
        assert!(rendered.contains("1 assets"));
        assert!(rendered.contains("api"));
        assert!(rendered.contains(r#"class="danger" type="submit" disabled"#));
        assert!(rendered.contains("window.openCredentialEditor"));
        assert!(rendered.contains("window.hopToggleCredentialSecrets"));
    }

    #[test]
    fn credential_fallback_edit_never_renders_existing_secrets() {
        let credential = credential("cred-1", "prod-root", "root", "password");
        let rendered = edit_credential(&EN, &credential, &[], "csrf-123").into_string();

        assert!(rendered.contains("Encrypted value stored"));
        assert!(rendered.contains(r#"name="password" autocomplete="new-password""#));
        assert!(!rendered.contains("encrypted-password"));
        assert!(rendered.contains(EN.secret_keep_note));
    }

    #[test]
    fn known_hosts_page_explains_and_confirms_trust_reset() {
        let host = KnownHost {
            hostname: "api.internal".to_string(),
            port: 22,
            key_type: "ssh-ed25519".to_string(),
            fingerprint: "SHA256:trusted".to_string(),
            first_seen: Some("2026-07-28 10:00:00".to_string()),
        };
        let assigned_asset = asset("api", "api.internal", &["prod"]);
        let rendered = known_hosts(&EN, &[host], &[assigned_asset], "csrf-123", true).into_string();

        assert!(rendered.contains(EN.known_hosts_reset_note));
        assert!(rendered.contains("1 assets"));
        assert!(rendered.contains(r#"data-copy-value="SHA256:trusted""#));
        assert!(rendered.contains(r#"aria-controls="reset-known-host-dialog""#));
        assert!(rendered.contains(r#"<dialog class="confirm-dialog" id="reset-known-host-dialog""#));
        assert!(rendered.contains(r#"name="confirm_reset" value="yes""#));
        assert!(rendered.contains(r#"data-reset-submit disabled"#));
        assert!(rendered.contains("window.openKnownHostReset"));
        assert!(rendered.contains("window.copyKnownHostFingerprint"));
    }

    #[test]
    fn sessions_page_renders_as_audit_replay_console() {
        let session_items = vec![Session {
            id: "session-1".to_string(),
            key_finger: "SHA256:test".to_string(),
            key_name: Some("alice".to_string()),
            mode: "direct".to_string(),
            asset_name: Some("prod-api-01".to_string()),
            target_host: Some("10.42.1.12".to_string()),
            target_port: Some(22),
            client_ip: Some("10.42.0.18".to_string()),
            status: "failed".to_string(),
            error: Some("password rejected".to_string()),
            started_at: Some("2026-06-17T14:39:12Z".to_string()),
            ended_at: None,
        }];
        let admin_events = vec![AuditEvent {
            id: "audit-1".to_string(),
            occurred_at: Some("2026-06-17T14:40:00Z".to_string()),
            actor_id: Some("local-admin".to_string()),
            actor_label: "Local admin".to_string(),
            action: "asset.update".to_string(),
            target_type: "asset".to_string(),
            target_id: Some("prod-api-01".to_string()),
            target_label: Some("prod-api-01".to_string()),
            result: "success".to_string(),
            source_ip: Some("127.0.0.1".to_string()),
            details_json: Some(r#"{"protocol":"ssh"}"#.to_string()),
        }];
        let rendered =
            sessions(&EN, &session_items, &admin_events, &[], "csrf-123", false).into_string();

        assert!(rendered.contains(r#"class="audit-page""#));
        assert!(rendered.contains("Audit Logs"));
        assert!(rendered.contains("Administrative changes and SSH access"));
        assert!(rendered.contains("hop admin sessions --limit 100"));
        assert!(rendered.contains("direct"));
        assert!(rendered.contains("asset.update"));
        assert!(rendered.contains("Local admin"));
        assert!(rendered.contains("password rejected"));
        assert!(!rendered.contains("AUTH_FAIL"));
        assert!(!rendered.contains("Replay: latest SSH trace"));
        assert!(!rendered.contains("sudo systemctl reload postgres"));
        assert!(!rendered.contains("Policy Feed"));
    }

    #[test]
    fn sessions_page_exposes_controls_for_registered_live_sessions() {
        let session_items = vec![Session {
            id: "session-live".to_string(),
            key_finger: "SHA256:test".to_string(),
            key_name: Some("alice".to_string()),
            mode: "exec".to_string(),
            asset_name: Some("prod-api-01".to_string()),
            target_host: Some("10.42.1.12".to_string()),
            target_port: Some(22),
            client_ip: Some("10.42.0.18".to_string()),
            status: "started".to_string(),
            error: None,
            started_at: Some("2026-06-17T14:39:12Z".to_string()),
            ended_at: None,
        }];
        let rendered = sessions(
            &EN,
            &session_items,
            &[],
            &["session-live".to_string()],
            "csrf-123",
            true,
        )
        .into_string();

        assert!(rendered.contains(r#"action="/sessions/terminate-all""#));
        assert!(rendered.contains(r#"action="/sessions/session-live/terminate""#));
        assert!(rendered.contains(r#"name="csrf_token" value="csrf-123""#));
        assert!(rendered.contains(EN.sessions_terminate));
    }

    #[test]
    fn assets_page_renders_protocol_controls_and_rdp_tunnel_hint() {
        let mut rdp = asset("win-rdp", "10.0.2.20", &["windows"]);
        rdp.protocol = ASSET_PROTOCOL_TCP.to_string();
        rdp.preset = Some(ASSET_PRESET_RDP.to_string());
        rdp.port = 3389;

        let rendered =
            assets(&EN, &[rdp], &[], "csrf-123", None, None, &[], 2222, true).into_string();

        assert!(rendered.contains(r#"name="protocol""#));
        assert!(rendered.contains(r#"value="rdp""#));
        assert!(rendered.contains("RDP"));
        assert!(rendered.contains(r#"data-rdp-port-hint"#));
        assert!(rendered.contains("3390"));
        assert!(rendered.contains("ssh -p 2222 -N -T -L 127.0.0.1:13389:win-rdp.hop:3389 hop-host"));
    }

    #[test]
    fn assets_page_renders_generic_tcp_presets_with_shared_tunnel_transport() {
        let cases = [
            (ASSET_PRESET_VNC, 5900, 15900),
            (ASSET_PRESET_MYSQL, 3306, 13306),
            (ASSET_PRESET_POSTGRES, 5432, 15432),
            (ASSET_PRESET_REDIS, 6379, 16379),
        ];

        for (preset, remote_port, local_port) in cases {
            let mut item = asset(preset, "10.0.0.20", &[]);
            item.protocol = ASSET_PROTOCOL_TCP.to_string();
            item.preset = Some(preset.to_string());
            item.port = remote_port;
            let rendered =
                assets(&EN, &[item], &[], "csrf-123", None, None, &[], 2222, true).into_string();

            assert!(rendered.contains(&format!(r#"value="{preset}""#)));
            assert!(rendered.contains(&format!(
                "ssh -p 2222 -N -T -L 127.0.0.1:{local_port}:{preset}.hop:{remote_port} hop-host"
            )));
        }
    }

    #[test]
    fn settings_page_renders_admin_password_form() {
        let current = admin_user("local-admin", "admin", ADMIN_PROFILE_OWNER, true);
        let rendered = settings(
            &EN,
            &current,
            std::slice::from_ref(&current),
            "csrf-123",
            Some("problem"),
            true,
        )
        .into_string();

        assert!(rendered.contains(r#"action="/settings""#));
        assert!(rendered.contains(r#"name="current_password""#));
        assert!(rendered.contains(r#"name="new_password""#));
        assert!(rendered.contains(r#"name="confirm_password""#));
        assert!(rendered.contains(r#"value="csrf-123""#));
        assert!(rendered.contains("problem"));
        assert!(rendered.contains(r#"id="add-admin-dialog""#));
        assert!(rendered.contains(EN.admin_single_mode));
        assert!(rendered.contains(r#"name="access_profile""#));
        assert!(!rendered.contains("RBAC"));
    }

    #[test]
    fn settings_progressively_shows_team_accounts_and_hides_management_for_viewers() {
        let owner = admin_user("local-admin", "admin", ADMIN_PROFILE_OWNER, true);
        let viewer = admin_user("viewer-1", "auditor", ADMIN_PROFILE_VIEWER, true);
        let rendered = settings(
            &EN,
            &viewer,
            &[owner, viewer.clone()],
            "csrf-123",
            None,
            false,
        )
        .into_string();

        assert!(rendered.contains(EN.admin_team_mode));
        assert!(rendered.contains("@auditor"));
        assert!(rendered.contains(EN.admin_profile_viewer));
        assert!(!rendered.contains(r#"id="add-admin-dialog""#));
        assert!(!rendered.contains(r#"action="/settings/admins""#));
    }

    #[test]
    fn access_page_creates_scope_in_the_same_lightweight_flow() {
        let first = asset("first", "10.0.0.1", &["prod"]);
        let rendered = keys(&EN, &[], &[first], "csrf-123", None, true).into_string();

        assert!(rendered.contains("People &amp; SSH access"));
        assert!(rendered.contains(r#"name="asset_access_mode""#));
        assert!(rendered.contains(r#"name="asset_id""#));
        assert!(rendered.contains(EN.key_access_all));
        assert!(rendered.contains(EN.key_access_restricted));
        assert!(!rendered.contains("RBAC"));
    }

    #[test]
    fn read_only_pages_hide_mutating_affordances_but_keep_safe_actions() {
        let item = asset("first", "10.0.0.1", &["prod"]);
        let asset_page = assets(
            &EN,
            std::slice::from_ref(&item),
            &[],
            "csrf-123",
            None,
            None,
            &["prod".to_string()],
            2222,
            false,
        )
        .into_string();
        assert!(asset_page.contains(EN.asset_copy_target));
        assert!(asset_page.contains(EN.export_csv));
        assert!(!asset_page.contains(r#"id="open-add-asset""#));
        assert!(!asset_page.contains(r#"type="checkbox" name="asset_ids""#));
        assert!(!asset_page.contains(r#"id="edit-asset-first""#));
        assert!(!asset_page.contains(r#"formaction="/assets/first/delete""#));
        assert!(!asset_page.contains(r#"class="ghost-button" href="/import""#));

        let credential_page = credentials(
            &EN,
            &[credential("cred-1", "prod-root", "root", "password")],
            &[],
            "csrf-123",
            None,
            false,
        )
        .into_string();
        assert!(credential_page.contains(EN.export_csv));
        assert!(!credential_page.contains(r#"id="open-add-credential""#));
        assert!(!credential_page.contains(r#"id="edit-credential-cred-1""#));
        assert!(!credential_page.contains(r#"/credentials/cred-1/delete"#));

        let key_page = keys(
            &EN,
            &[authorized_key(AssetAccessMode::All)],
            &[item],
            "csrf-123",
            None,
            false,
        )
        .into_string();
        assert!(key_page.contains(EN.keys_existing_heading));
        assert!(!key_page.contains(r#"action="/keys""#));
        assert!(!key_page.contains("/edit"));
        assert!(!key_page.contains("/deactivate"));

        let host = KnownHost {
            hostname: "10.0.0.1".to_string(),
            port: 22,
            key_type: "ssh-ed25519".to_string(),
            fingerprint: "SHA256:trusted".to_string(),
            first_seen: None,
        };
        let host_page = known_hosts(&EN, &[host], &[], "csrf-123", false).into_string();
        assert!(host_page.contains(EN.known_host_copy_fingerprint));
        assert!(!host_page.contains(r#"aria-controls="reset-known-host-dialog""#));
    }

    #[test]
    fn permission_request_copy_uses_locale_appropriate_spacing_and_punctuation() {
        let english = permission_denied(&EN, EN.nav_assets, ADMIN_PROFILE_VIEWER).into_string();
        let chinese = permission_denied(&ZH, ZH.nav_assets, ADMIN_PROFILE_VIEWER).into_string();

        assert!(english.contains("Request: please allow me to manage Assets."));
        assert!(chinese.contains("请求文案：请允许我管理资产。"));
    }

    #[test]
    fn key_edit_page_renders_access_modes_search_and_assignments() {
        let mut key = authorized_key(AssetAccessMode::Restricted);
        let first = asset("first", "10.0.0.1", &["prod"]);
        let second = asset("second", "10.0.0.2", &[]);
        let rendered = edit_key(
            &EN,
            &key,
            &[first.clone(), second],
            std::slice::from_ref(&first.id),
            "csrf-123",
            Some("validation failed"),
        )
        .into_string();

        assert!(rendered.contains(r#"name="asset_access_mode""#));
        assert!(rendered.contains(r#"value="all""#));
        assert!(rendered.contains(r#"value="restricted" selected"#));
        assert!(rendered.contains(r#"type="search""#));
        assert!(rendered.contains(&format!(r#"value="{}" checked"#, first.id)));
        assert!(rendered.contains("1 / 2 assets"));
        assert!(rendered.contains("validation failed"));
        assert!(rendered.contains(r#"name="csrf_token" value="csrf-123""#));

        key.asset_access_mode = AssetAccessMode::All;
        let all_rendered = edit_key(&EN, &key, &[first], &[], "csrf-123", None).into_string();
        assert!(all_rendered.contains(r#"value="all" selected"#));
        assert!(all_rendered.contains("1 / 1 assets"));
        assert!(all_rendered.contains("Current and future assets are automatically accessible."));
    }

    #[test]
    fn import_page_uses_multipart_upload_form() {
        let rendered = import_export(&EN, "csrf-123", None).into_string();

        assert!(rendered.contains(r#"enctype="multipart/form-data""#));
        assert!(rendered.contains(r#"type="file""#));
    }

    fn asset(name: &str, hostname: &str, tags: &[&str]) -> Asset {
        Asset {
            id: name.to_string(),
            name: name.to_string(),
            protocol: ASSET_PROTOCOL_SSH.to_string(),
            preset: None,
            hostname: hostname.to_string(),
            port: 22,
            description: None,
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
            credential_id: None,
            created_at: None,
            updated_at: None,
        }
    }

    fn credential(id: &str, name: &str, username: &str, auth_type: &str) -> Credential {
        Credential {
            id: id.to_string(),
            name: name.to_string(),
            username: username.to_string(),
            auth_type: auth_type.to_string(),
            password_enc: Some("encrypted-password".to_string()),
            private_key_enc: None,
            passphrase_enc: None,
            created_at: None,
        }
    }

    fn authorized_key(mode: AssetAccessMode) -> AuthorizedKey {
        AuthorizedKey {
            id: "key-1".to_string(),
            name: "laptop".to_string(),
            public_key: "ssh-ed25519 AAAA".to_string(),
            fingerprint: "SHA256:test".to_string(),
            is_active: true,
            asset_access_mode: mode,
            created_at: None,
        }
    }

    fn admin_user(id: &str, username: &str, access_profile: &str, is_active: bool) -> AdminUser {
        AdminUser {
            id: id.to_string(),
            username: username.to_string(),
            display_name: if id == "local-admin" {
                "Local admin".to_string()
            } else {
                username.to_string()
            },
            auth_source: "local".to_string(),
            is_active,
            access_profile: access_profile.to_string(),
            must_change_password: false,
            created_at: Some("2026-07-28 10:00:00".to_string()),
            last_login_at: None,
        }
    }

    fn dashboard_data() -> DashboardData {
        let mut managed = asset("managed-prod", "managed.prod.internal", &["prod"]);
        managed.credential_id = Some("credential-1".to_string());
        let unmanaged = asset("unmanaged-stage", "stage.internal", &["stage"]);
        DashboardData {
            gateway: DashboardGateway {
                admin_bind: "127.0.0.1:8080".to_string(),
                ssh_bind: "127.0.0.1:2222".to_string(),
                version: "0.1.5".to_string(),
                started_at: "2026-07-28T10:00:00Z".to_string(),
                uptime_seconds: 3_661,
                admin_reachable: true,
                ssh_reachable: true,
                database_healthy: true,
            },
            assets: vec![managed.clone(), unmanaged.clone()],
            credentials: vec![credential("credential-1", "production", "root", "password")],
            keys: vec![AuthorizedKey {
                id: "key-1".to_string(),
                name: "Alice laptop".to_string(),
                public_key: "ssh-ed25519 AAAA".to_string(),
                fingerprint: "SHA256:alice".to_string(),
                is_active: true,
                asset_access_mode: AssetAccessMode::Restricted,
                created_at: Some("2026-07-28 10:00:00".to_string()),
            }],
            known_hosts: vec![KnownHost {
                hostname: managed.hostname.clone(),
                port: managed.port,
                key_type: "ssh-ed25519".to_string(),
                fingerprint: "SHA256:managed".to_string(),
                first_seen: Some("2026-07-28 10:00:00".to_string()),
            }],
            asset_health: vec![
                AssetHealth {
                    asset_id: managed.id.clone(),
                    status: ASSET_HEALTH_HEALTHY.to_string(),
                    checked_at: Some("2026-07-28 10:00:00".to_string()),
                    last_success_at: Some("2026-07-28 10:00:00".to_string()),
                    latency_ms: Some(42),
                    error_code: None,
                    error_message: None,
                },
                AssetHealth {
                    asset_id: unmanaged.id.clone(),
                    status: ASSET_HEALTH_UNKNOWN.to_string(),
                    checked_at: None,
                    last_success_at: None,
                    latency_ms: None,
                    error_code: None,
                    error_message: None,
                },
            ],
            recent_sessions: vec![Session {
                id: "session-1".to_string(),
                key_finger: "SHA256:alice".to_string(),
                key_name: Some("Alice laptop".to_string()),
                mode: "direct".to_string(),
                asset_name: Some(managed.name.clone()),
                target_host: Some(managed.hostname.clone()),
                target_port: Some(managed.port),
                client_ip: Some("127.0.0.1".to_string()),
                status: "ok".to_string(),
                error: None,
                started_at: Some("2026-07-28 10:00:00".to_string()),
                ended_at: Some("2026-07-28 10:00:08".to_string()),
            }],
            sessions_24h: 5,
            recent_admin_events: vec![AuditEvent {
                id: "event-1".to_string(),
                occurred_at: Some("2026-07-28 10:01:00".to_string()),
                actor_id: Some("local-admin".to_string()),
                actor_label: "Local admin".to_string(),
                action: "asset.update".to_string(),
                target_type: "asset".to_string(),
                target_id: Some(managed.id.clone()),
                target_label: Some(managed.name.clone()),
                result: "success".to_string(),
                source_ip: Some("127.0.0.1".to_string()),
                details_json: None,
            }],
            source_errors: Vec::new(),
        }
    }
}
