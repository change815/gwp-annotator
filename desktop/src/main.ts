import "./style.css";

const app = document.querySelector<HTMLDivElement>("#app");

if (!app) {
  throw new Error("Application mount point is unavailable.");
}

app.innerHTML = `
  <main aria-labelledby="app-title">
    <h1 id="app-title">GWP Annotator</h1>
    <p>Local core is ready. The clinician interface is delivered in a later stage.</p>
    <p class="status" data-testid="offline-status">Offline by default</p>
  </main>
`;

