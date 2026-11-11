// A headless check of the map page's state: ticked crates and the panel's
// crate are kept by name through every filter and a switch of map, and Reset
// view clears them. It needs Playwright with Chromium; CI does not run it.
//
//   npx -p playwright playwright install chromium   (once)
//   npx -p playwright node docs/architecture/page-test.js
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
    const same = async (label, ticked, panel) => {
      const got = await read();
      check(label, got.ticked === ticked && got.tickedClass === ticked && got.panel === panel, got);
    };

    for (const mode of ["current", "proposed"]) {
      if (mode === "proposed") await p.click('input[value="proposed"]');
      const m = (label) => `${mode}: ${label}`;
      check(m("nothing ticked, every arrow faint"), (await p.$$eval("path.edge:not(.faint)", (x) => x.length)) === 0);

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

      // Every filter, then undone.
      await p.click("#crates-button");
      await p.click("#kind-filters input >> nth=1");
      await same(m("a kind hidden"), "kkt-server", panel);
      await p.click("#kind-filters input >> nth=1");
      await same(m("the kind back"), both, panel);
      await p.click("#show-dev-crates");
      await p.click("#show-dev-crates");
      await p.keyboard.press("Escape");
      await same(m("dev-only off and on"), both, panel);
      await p.click("#arrows-button");
      await p.click('#role-filters input[data-key="role:uses"]');
      await same(m("a role off"), both, panel);
      await p.click('#role-filters input[data-key="role:uses"]');
      await p.click("#show-dev");
      await same(m("test arrows on"), both, panel);
      await p.click("#show-dev");
      await p.keyboard.press("Escape");
      await p.click(".band-toggle >> nth=1");
      await same(m("a layer hidden"), both, panel);
      await p.click(".band-toggle >> nth=1");
      await p.click(`${node("kashkeshet")} .eye-hit`, { force: true });
      await same(m("a program hidden"), both, panel);
      await p.click(`${node("kashkeshet")} .eye-hit`, { force: true });
      await p.click("#crates-button");
      await p.click("#crates-none");
      await p.click("#crates-all");
      await p.keyboard.press("Escape");
      await same(m("crates none, then all"), both, panel);
      await p.click("#layers-hide");
      await p.click("#layers-show");
      await same(m("layers hidden, then shown"), both, panel);

      await p.click("#reset-view");
      const reset = await read();
      check(m("reset clears ticks and panel"), reset.ticked === "" && reset.panel !== panel, reset);
    }

    // A crate ticked in one map and missing from the other stays ticked, is
    // listed without a link there, and its panel cannot be opened.
    await p.click('input[value="current"]');
    await p.click(`${node("kkt-config")} .pick-hit`, { force: true });
    await p.click('input[value="proposed"]');
    const other = await read();
    check("a crate only the current map has, in the proposed map",
      other.list.includes("kkt-config (not in this map)") &&
      (await p.$$eval(".selection-list button.crate-link", (x) => x.length)) === 0, other);
    await p.click('input[value="current"]');
    check("back in the current map, it is still ticked", (await read()).ticked === "kkt-config");
    await p.click("#reset-view");
    await context.close();
  }
  console.log(`errors: ${errors.length ? errors.join("; ") : "none"}; failures: ${failures}`);
  await browser.close();
  process.exit(failures || errors.length ? 1 : 0);
})();
