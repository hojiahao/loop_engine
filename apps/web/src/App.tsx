export function App() {
  return (
    <div className="shell">
      <header className="topbar">
        <strong>Loop Engine</strong>
        <span className="environment">Local research</span>
      </header>
      <main className="workspace">
        <section aria-labelledby="control-plane-title">
          <p className="eyebrow">SYSTEM STATUS</p>
          <h1 id="control-plane-title">Control plane</h1>
          <dl className="status-grid">
            <div>
              <dt>Loop runtime</dt>
              <dd>Bootstrap ready</dd>
            </div>
            <div>
              <dt>Research data</dt>
              <dd>Not configured</dd>
            </div>
            <div>
              <dt>Model providers</dt>
              <dd>Not configured</dd>
            </div>
          </dl>
        </section>
      </main>
    </div>
  );
}
