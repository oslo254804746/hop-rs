import "./styles.css";

const app = document.querySelector<HTMLDivElement>("#app");

if (app) {
  app.innerHTML = `
    <section class="shell">
      <p class="eyebrow">Hop Admin Web</p>
      <h1>Static frontend pipeline ready</h1>
      <p>
        Build assets from this workspace and let hop-server serve the generated dist directory.
      </p>
    </section>
  `;
}
