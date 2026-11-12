// A headless check of the map page: ticked crates and the panel's crate are
// kept by name through every filter and a switch of map, and Reset view
// clears them; no box moves when a filter changes; and only full-strength
// arrows answer the pointer. It needs Playwright with Chromium; CI does not
// run it. From the repository's root:
//
//   npm install --prefix target/tmp/page-test playwright           (once)
//   target/tmp/page-test/node_modules/.bin/playwright install chromium  (once)
//   NODE_PATH=target/tmp/page-test/node_modules node docs/architecture/page-test.js
//
// It exits non-zero on a failed check or a page error.

const path = require("path");
const { chromium } = require("playwright");

const PAGE = "file://" + path.resolve(__dirname, "index.html");

(async () => {
  const browser = await chromium.launch();
  const errors = [];
  let failures = 0;
  for (const [width, height, scheme] of [[1400, 1000, "light"], [390, 844, "dark"]]) {
    const context = await browser.newContext({ viewport: { width, height }, colorScheme: scheme });
    const p = await context.newPage();
    p.on("pageerror", (e) => errors.push(e.message));
    p.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
    await p.goto(PAGE);

    const node = (name) => `.node[aria-label^='${name}:']`;
    const read = () => p.evaluate(() => ({
      ticked: [...document.querySelectorAll(".node")]
        .filter((n) => n.querySelector(".pick").getAttribute("aria-checked") === "true")
        .map((n) => n.getAttribute("aria-label").split(":")[0]).sort().join(","),
      tickedClass: [...document.querySelectorAll(".node.checked")]
        .map((n) => n.getAttribute("aria-label").split(":")[0]).sort().join(","),
      panel: document.querySelector("#panel h2").textContent,
      list: (document.querySelector(".selection-list") || { textContent: "" }).textContent,
    }));
    const check = (label, ok, got) => {
      if (!ok) failures++;
      console.log(`${width}/${scheme} ${label}: ${ok ? "ok" : "FAIL " + JSON.stringify(got)}`);
    };
    // Every drawn box's place, by name.
    const places = () => p.evaluate(() => Object.fromEntries([...document.querySelectorAll(".node")]
      .map((n) => [n.getAttribute("aria-label").split(":")[0], n.getAttribute("transform")])));
    const unmoved = async (label, before) => {
      const now = await places();
      const moved = Object.keys(now).filter((n) => before[n] && before[n] !== now[n]);
      const missing = Object.keys(now).filter((n) => !before[n]);
      check(`${label}: no box moved`, moved.length === 0 && missing.length === 0, { moved, missing });
    };
    const same = async (label, ticked, panel) => {
      const got = await read();
      check(label, got.ticked === ticked && got.tickedClass === ticked && got.panel === panel, got);
    };

    for (const mode of ["current", "proposed"]) {
      if (mode === "proposed") await p.click('input[value="proposed"]');
      const m = (label) => `${mode}: ${label}`;
      const home = await places();
      check(m("nothing ticked, every arrow faint"), (await p.$$eval("path.edge:not(.faint)", (x) => x.length)) === 0);

      // A merged crate's box is split, and an arrow that turns its `real`
      // on meets the implementation part, right of one that does not.
      const merged = "kkt-agent-client";
      check(m("a merged crate's box has its implementation part"),
        (await p.$$eval(`${node(merged)} .impl-part`, (x) => x.length)) === 1);
      const endX = (from) => p.$eval(`path.edge[data-edge='${from}>${merged}>normal'][data-head]`, (path) => {
        const box = path.ownerSVGElement.querySelector(`.node[aria-label^='${path.dataset.edge.split(">")[1]}:']`);
        const at = path.getPointAtLength(path.getTotalLength());
        return at.x - box.transform.baseVal[0].matrix.e;
      });
      const contractX = await endX("kkt-agent-commands");
      const implementationX = await endX("kashkeshet");
      check(m("a consumer's arrow meets the contract, the program's the implementation"),
        contractX < implementationX - 40, { contractX, implementationX });
      // A "uses" arrow's head is at the user: drawn from the used crate.
      const head = await p.$eval(`path.edge[data-edge='kkt-agent-commands>${merged}>normal'][data-head]`,
        (path) => [path.getAttribute("marker-start"), path.getAttribute("marker-end"), path.dataset.head]);
      check(m("a uses arrow points at the crate that uses"),
        head[0] === "url(#head-uses)" && head[1] === null && head[2] === "kkt-agent-commands", head);
      // Its panel lists the two parts, and which crates include each.
      await p.click(`${node(merged)} > rect`);
      const parts = await p.$eval("#panel", (panel) => panel.innerText);
      check(m("a merged crate's panel lists its two parts"),
        /Contract \(\d+\): .*Servers/.test(parts) && /Implementation, behind real \(\d+\): .*Mcp/.test(parts)
          && /turning real on: .*kashkeshet/.test(parts) && /the contract only: .*kkt-agent-commands/.test(parts), parts.slice(0, 200));

      // Clicking a box shows it in the panel and highlights nothing.
      await p.click(`${node("kkt-hosts")} > rect`);
      const panel = (await read()).panel;
      check(m("click shows the panel, no highlight"),
        panel === "kkt-hosts" && (await p.$$eval("path.edge.focus", (x) => x.length)) === 0);

      // Ticking highlights and leaves the panel alone.
      await p.click(`${node("kkt-setup")} .pick-hit`, { force: true });
      await p.click(`${node("kkt-server")} .pick-hit`, { force: true });
      const both = "kkt-server,kkt-setup";
      await same(m("tick two"), both, panel);
      check(m("ticked crates light up"), (await p.$$eval("path.edge.focus", (x) => x.length)) > 0);
      await unmoved(m("ticks and a click"), home);

      // Only full-strength arrows answer the pointer: a faded arrow's hover
      // area takes none, and hovering a highlighted arrow shows its tooltip.
      const pointer = await p.evaluate(() => {
        const hits = [...document.querySelectorAll("path.edge-hit")];
        const events = (h) => getComputedStyle(h).pointerEvents;
        return { faint: hits.filter((h) => h.classList.contains("faint")).map(events),
                 full: hits.filter((h) => !h.classList.contains("faint")).map(events) };
      });
      check(m("faded arrows take no pointer"), pointer.faint.length > 0 && pointer.faint.every((e) => e === "none"), pointer.faint.slice(0, 3));
      check(m("highlighted arrows take the pointer"), pointer.full.length > 0 && pointer.full.every((e) => e !== "none"), pointer.full.slice(0, 3));
      const spot = (sel) => p.evaluate((s) => {
        const svgEl = document.getElementById("graph");
        for (const h of document.querySelectorAll(s)) {
          const len = h.getTotalLength();
          for (const f of [0.5, 0.4, 0.6, 0.3, 0.7]) {
            const pt = svgEl.createSVGPoint();
            const q = h.getPointAtLength(len * f);
            pt.x = q.x; pt.y = q.y;
            const sc = pt.matrixTransform(svgEl.getScreenCTM());
            const top = document.elementFromPoint(sc.x, sc.y);
            if (top && (top === svgEl || top.classList.contains("band") || top === h)) return { x: sc.x, y: sc.y, top: top.getAttribute("class") };
          }
        }
        return null;
      }, sel);
      await p.mouse.move(5, 5);
      const full = await spot("path.edge-hit:not(.faint)");
      if (full) await p.mouse.move(full.x, full.y);
      check(m("hovering a highlighted arrow shows its tooltip"), Boolean(full) && await p.isVisible("#tip"), full);
      await p.mouse.move(5, 5);
      const faint = await spot("path.edge-hit.faint");
      if (faint) await p.mouse.move(faint.x, faint.y);
      check(m("hovering a faded arrow shows nothing"), Boolean(faint) && !(await p.isVisible("#tip")), faint);
      await p.mouse.move(5, 5);

      // Every filter, then undone.
      await p.click("#crates-button");
      // The logic kind, by name: which kinds are listed, and in what order,
      // follows the crates the map has.
      const logic = "#kind-filters input[data-key='kind:logic']";
      await p.click(logic);
      await same(m("a kind hidden"), "kkt-server", panel);
      await unmoved(m("a kind hidden"), home);
      await p.click(logic);
      await same(m("the kind back"), both, panel);
      await unmoved(m("the kind back"), home);
      await p.click("#show-dev-crates");
      await unmoved(m("dev-only off"), home);
      await p.click("#show-dev-crates");
      await p.keyboard.press("Escape");
      await same(m("dev-only off and on"), both, panel);
      await unmoved(m("dev-only off and on"), home);
      await p.click("#arrows-button");
      await p.click('#role-filters input[data-key="role:uses"]');
      await same(m("a role off"), both, panel);
      await unmoved(m("a role off"), home);
      await p.click('#role-filters input[data-key="role:uses"]');
      await p.click("#show-dev");
      await same(m("test arrows on"), both, panel);
      await unmoved(m("test arrows on"), home);
      await p.click("#show-dev");
      await p.keyboard.press("Escape");
      await p.click(".band-toggle >> nth=1");
      await same(m("a layer hidden"), both, panel);
      await unmoved(m("a layer hidden"), home);
      await p.click(".band-toggle >> nth=1");
      await p.click(`${node("kashkeshet")} .eye-hit`, { force: true });
      await same(m("a program hidden"), both, panel);
      await unmoved(m("a program hidden"), home);
      await p.click(`${node("kashkeshet")} .eye-hit`, { force: true });
      await p.click("#crates-button");
      await p.click("#crates-none");
      await p.click("#crates-all");
      await p.keyboard.press("Escape");
      await same(m("crates none, then all"), both, panel);
      await unmoved(m("crates none, then all"), home);
      await p.click("#layers-hide");
      await p.click("#layers-show");
      await same(m("layers hidden, then shown"), both, panel);
      await unmoved(m("layers hidden, then shown"), home);

      await p.click("#reset-view");
      const reset = await read();
      check(m("reset clears ticks and panel"), reset.ticked === "" && reset.panel !== panel, reset);
    }

    // A crate ticked in one map and missing from the other stays ticked, is
    // listed without a link there, and its panel cannot be opened. Since the
    // refactor, both maps may have the same crates; the check then says so.
    const only = await p.evaluate(() => {
      const names = (g) => new Set(g.crates.map((c) => c.name));
      const cur = names(window.KKT_GRAPH), pro = names(window.KKT_PROPOSED.graph);
      const a = [...cur].find((n) => !pro.has(n));
      if (a) return { name: a, from: "current", to: "proposed" };
      const b = [...pro].find((n) => !cur.has(n));
      return b ? { name: b, from: "proposed", to: "current" } : null;
    });
    if (only) {
      await p.click(`input[value="${only.from}"]`);
      await p.click(`${node(only.name)} .pick-hit`, { force: true });
      await p.click(`input[value="${only.to}"]`);
      const other = await read();
      check(`a crate only the ${only.from} map has, in the ${only.to} map`,
        other.list.includes(`${only.name} (not in this map)`) &&
        (await p.$$eval(".selection-list button.crate-link", (x) => x.length)) === 0, other);
      await p.click(`input[value="${only.from}"]`);
      check(`back in the ${only.from} map, it is still ticked`, (await read()).ticked === only.name);
    } else {
      console.log(`${width}/${scheme} a crate only one map has: skipped, both maps have the same crates`);
    }
    await p.click("#reset-view");
    await context.close();
  }
  console.log(`errors: ${errors.length ? errors.join("; ") : "none"}; failures: ${failures}`);
  await browser.close();
  process.exit(failures || errors.length ? 1 : 0);
})();
