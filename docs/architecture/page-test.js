// A headless check of the map page: ticked crates and the panel's crate are
// kept by name through every filter, and Reset view clears them; no box
// moves when a filter changes; and only full-strength arrows answer the
// pointer. It needs Playwright with Chromium; CI does not run it. From the repository's root:
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

    // The page shows the code only: no switch of map, and it loads no
    // proposal.
    check("no switch of map, no proposal loaded",
      (await p.$$eval('input[name="mode"]', (x) => x.length)) === 0 &&
      (await p.evaluate(() => window.KKT_PROPOSED === undefined)));
    // The overview reports the rule checks, computed on the code, after
    // the kind checks.
    const overview = await p.$eval("#panel", (panel) => panel.textContent);
    check("the overview has kind checks, then rule checks",
      /Kind checks \(\d+ to look at\)[\s\S]*Rule checks \(\d+ broken\)/.test(overview), overview.slice(0, 300));

    const home = await places();
    check("nothing ticked, every arrow faint", (await p.$$eval("path.edge:not(.faint)", (x) => x.length)) === 0);

    // A merged crate's box is split, and an arrow that turns its `real`
    // on meets the implementation part, right of one that does not.
    const merged = "kkt-agent-client";
    check("a merged crate's box has its implementation part",
      (await p.$$eval(`${node(merged)} .impl-part`, (x) => x.length)) === 1);
    // Every merged crate's name ends before the divider, and the green
    // mark on the divider sits below it.
    const overruns = await p.$$eval(".node .impl-divider", (dividers) => dividers.map((d) => {
      const name = d.parentNode.querySelector("text.name").getBBox();
      const mark = d.parentNode.querySelector(".self-implements").getBBox();
      return [d.parentNode.getAttribute("aria-label").split(":")[0], name.x + name.width, d.getBBox().x, name.y + name.height, mark.y];
    }).filter(([, end, divider, bottom, markTop]) => end > divider || markTop < bottom));
    check("each merged crate's name clears the divider and its mark", overruns.length === 0, overruns);
    const endX = (from) => p.$eval(`path.edge[data-edge='${from}>${merged}>normal'][data-head]`, (path) => {
      const box = path.ownerSVGElement.querySelector(`.node[aria-label^='${path.dataset.edge.split(">")[1]}:']`);
      const at = path.getPointAtLength(path.getTotalLength());
      return at.x - box.transform.baseVal[0].matrix.e;
    });
    const contractX = await endX("kkt-agent-commands");
    const implementationX = await endX("kashkeshet");
    check("a consumer's arrow meets the contract, the program's the implementation",
      contractX < implementationX - 40, { contractX, implementationX });
    // Each stroke meets the part its role depends on: the program's
    // arrow to kkt-container implements its contract (the program's
    // adapters) and wires its implementation.
    const strokes = await p.$$eval("path.edge[data-edge='kashkeshet>kkt-container>normal']", (paths) => paths.map((path) => {
      const box = path.ownerSVGElement.querySelector(".node[aria-label^='kkt-container:']");
      const end = path.getPointAtLength(path.getTotalLength()).x - box.transform.baseVal[0].matrix.e;
      return [[...path.classList].find((c) => c.startsWith("role-")), Math.round(end)];
    }));
    const at = (role) => (strokes.find(([r]) => r === role) || [])[1];
    check("an implementing stroke meets the contract, a wiring one the implementation",
      at("role-implements") < 110 && at("role-wires") > 122, strokes);
    // A "uses" arrow's head is at the user: drawn from the used crate.
    const head = await p.$eval(`path.edge[data-edge='kkt-agent-commands>${merged}>normal'][data-head]`,
      (path) => [path.getAttribute("marker-start"), path.getAttribute("marker-end"), path.dataset.head]);
    check("a uses arrow points at the crate that uses",
      head[0] === "url(#head-uses)" && head[1] === null && head[2] === "kkt-agent-commands", head);
    // Its panel lists the two parts, and which crates include each.
    await p.click(`${node(merged)} > rect`);
    const parts = await p.$eval("#panel", (panel) => panel.innerText);
    check("a merged crate's panel lists its two parts",
      /Contract \(\d+\): .*Servers/.test(parts) && /Implementation, behind real \(\d+\): .*Mcp/.test(parts)
        && /turning real on: .*kashkeshet/.test(parts) && /the contract only: .*kkt-agent-commands/.test(parts), parts.slice(0, 200));

    // A ticked merged crate shows its delivery line, clear of the
    // highlighted arrows: kashkeshet sits just above kkt-agent-commands,
    // and a line meeting the boxes where the arrows do lay hidden on them.
    await p.click(`${node(merged)} .pick-hit`, { force: true });
    const clear = await p.evaluate(() => {
      const ends = (path) => [path.getPointAtLength(0), path.getPointAtLength(path.getTotalLength())];
      const line = document.querySelector("path.delivery[data-from='kashkeshet'][data-to='kkt-agent-commands']");
      if (!line) return { drawn: false };
      const arrows = [...document.querySelectorAll("path.edge.focus")].flatMap(ends);
      const near = ends(line).filter((q) => arrows.some((r) => Math.hypot(q.x - r.x, q.y - r.y) < 6));
      return { drawn: true, near: near.length };
    });
    check("a ticked merged crate's delivery line is drawn clear of the arrows", clear.drawn && clear.near === 0, clear);
    await p.click(`${node(merged)} .pick-hit`, { force: true });

    // Injections follow the rule (msg 3343): the program injects nothing of
    // kkt-diagnostics' own into it; kkt-operate passes on the prober it was
    // given; and the program's own work is labelled as such.
    await p.click(`${node("kkt-diagnostics")} .pick-hit`, { force: true });
    const lines = await p.$$eval("path.delivery", (paths) => paths.map((x) => `${x.dataset.from}>${x.dataset.to}`));
    check("no injection of a crate's own type, and the assembly passes on what it was given",
      !lines.includes("kashkeshet>kkt-diagnostics") && lines.includes("kkt-operate>kkt-diagnostics"), lines);
    await p.click(`${node("kkt-diagnostics")} .pick-hit`, { force: true });
    await p.click(`${node("kkt-operate")} .pick-hit`, { force: true });
    const labels = await p.$$eval("text.delivery-label", (xs) => xs.map((x) => x.textContent));
    check("the program's own work is named as such", labels.some((l) => l.includes("its own work")), labels);
    await p.click(`${node("kkt-operate")} .pick-hit`, { force: true });

    // Clicking a box shows it in the panel and highlights nothing.
    await p.click(`${node("kkt-hosts")} > rect`);
    const panel = (await read()).panel;
    check("click shows the panel, no highlight",
      panel === "kkt-hosts" && (await p.$$eval("path.edge.focus", (x) => x.length)) === 0);

    // Ticking highlights and leaves the panel alone.
    await p.click(`${node("kkt-watch")} .pick-hit`, { force: true });
    await p.click(`${node("kkt-server")} .pick-hit`, { force: true });
    const both = "kkt-server,kkt-watch";
    await same("tick two", both, panel);
    check("ticked crates light up", (await p.$$eval("path.edge.focus", (x) => x.length)) > 0);
    await unmoved("ticks and a click", home);

    // Only full-strength arrows answer the pointer: a faded arrow's hover
    // area takes none, and hovering a highlighted arrow shows its tooltip.
    const pointer = await p.evaluate(() => {
      const hits = [...document.querySelectorAll("path.edge-hit")];
      const events = (h) => getComputedStyle(h).pointerEvents;
      return { faint: hits.filter((h) => h.classList.contains("faint")).map(events),
               full: hits.filter((h) => !h.classList.contains("faint")).map(events) };
    });
    check("faded arrows take no pointer", pointer.faint.length > 0 && pointer.faint.every((e) => e === "none"), pointer.faint.slice(0, 3));
    check("highlighted arrows take the pointer", pointer.full.length > 0 && pointer.full.every((e) => e !== "none"), pointer.full.slice(0, 3));
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
    check("hovering a highlighted arrow shows its tooltip", Boolean(full) && await p.isVisible("#tip"), full);
    await p.mouse.move(5, 5);
    const faint = await spot("path.edge-hit.faint");
    if (faint) await p.mouse.move(faint.x, faint.y);
    check("hovering a faded arrow shows nothing", Boolean(faint) && !(await p.isVisible("#tip")), faint);
    await p.mouse.move(5, 5);

    // Every filter, then undone.
    await p.click("#crates-button");
    // The logic kind, by name: which kinds are listed, and in what order,
    // follows the crates the map has.
    const logic = "#kind-filters input[data-key='kind:logic']";
    await p.click(logic);
    await same("a kind hidden", "kkt-server", panel);
    await unmoved("a kind hidden", home);
    await p.click(logic);
    await same("the kind back", both, panel);
    await unmoved("the kind back", home);
    await p.click("#show-dev-crates");
    await unmoved("dev-only off", home);
    await p.click("#show-dev-crates");
    await p.keyboard.press("Escape");
    await same("dev-only off and on", both, panel);
    await unmoved("dev-only off and on", home);
    await p.click("#arrows-button");
    await p.click('#role-filters input[data-key="role:uses"]');
    await same("a role off", both, panel);
    await unmoved("a role off", home);
    await p.click('#role-filters input[data-key="role:uses"]');
    await p.click("#show-dev");
    await same("test arrows on", both, panel);
    await unmoved("test arrows on", home);
    await p.click("#show-dev");
    await p.keyboard.press("Escape");
    await p.click(".band-toggle >> nth=1");
    await same("a layer hidden", both, panel);
    await unmoved("a layer hidden", home);
    await p.click(".band-toggle >> nth=1");
    await p.click(`${node("kashkeshet")} .eye-hit`, { force: true });
    await same("a program hidden", both, panel);
    await unmoved("a program hidden", home);
    await p.click(`${node("kashkeshet")} .eye-hit`, { force: true });
    await p.click("#crates-button");
    await p.click("#crates-none");
    await p.click("#crates-all");
    await p.keyboard.press("Escape");
    await same("crates none, then all", both, panel);
    await unmoved("crates none, then all", home);
    await p.click("#layers-hide");
    await p.click("#layers-show");
    await same("layers hidden, then shown", both, panel);
    await unmoved("layers hidden, then shown", home);

    await p.click("#reset-view");
    const reset = await read();
    check("reset clears ticks and panel", reset.ticked === "" && reset.panel !== panel, reset);
    await context.close();
  }
  console.log(`errors: ${errors.length ? errors.join("; ") : "none"}; failures: ${failures}`);
  await browser.close();
  process.exit(failures || errors.length ? 1 : 0);
})();
