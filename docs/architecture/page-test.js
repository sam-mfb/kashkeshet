// A headless check of the map page: arrow heads on the crate that depends,
// none overlapping, and delivery lines leaving at points of their own;
// groups drawn beneath their heads and
// outlined, with every arrow ending on its own boxes; ticked crates and the panel's crate are
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
    // Clicks a part of a crate's box (its tick box, its eye, or the box),
    // first scrolling the box to the middle of the view, so a box near the
    // top is never under the sticky header and a click never lands there.
    // Every click on a box goes through it.
    const onBox = async (name, part) => {
      await p.$eval(node(name), (n) => n.scrollIntoView({ block: "center", inline: "center" }));
      // The tick box and the eye sit on the box's edge, where an arrow's
      // hover area can lie over them, so they need `force`. The box's body
      // does not, and keeps Playwright's check that nothing covers it.
      await p.click(`${node(name)} ${part}`, part === "> rect" ? {} : { force: true });
    };
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

    // Groups (generate.py's `groups`): each partner directly beneath its
    // head, the group outlined, and every arrow ending on its own crates'
    // boxes, never on another's.
    const groups = await p.evaluate(() => {
      const at = (name) => {
        const n = document.querySelector(`.node[aria-label^='${name}:']`);
        const m = /translate\(([-\d.]+),([-\d.]+)\)/.exec(n.getAttribute("transform"));
        return { x: +m[1], y: +m[2] };
      };
      const wrong = [];
      for (const [head, partners] of Object.entries(window.KKT_GRAPH.groups)) {
        let above = at(head);
        for (const name of partners) {
          const q = at(name);
          if (q.x !== above.x || q.y <= above.y) wrong.push(name);
          above = q;
        }
      }
      return { count: Object.keys(window.KKT_GRAPH.groups).length,
               outlines: document.querySelectorAll("rect.group").length, wrong };
    });
    check("each partner sits beneath its head, and each group is outlined",
      groups.count >= 9 && groups.outlines === groups.count && groups.wrong.length === 0, groups);
    const endsOnOwnBoxes = () => p.evaluate(() => {
      const boxes = [...document.querySelectorAll(".node")].map((n) => {
        const m = /translate\(([-\d.]+),([-\d.]+)\)/.exec(n.getAttribute("transform"));
        const r = n.querySelector("rect");
        return { name: n.getAttribute("aria-label").split(":")[0], x: +m[1], y: +m[2], w: +r.getAttribute("width"), h: +r.getAttribute("height") };
      });
      const near = (b, q, pad) => q.x >= b.x - pad && q.x <= b.x + b.w + pad && q.y >= b.y - pad && q.y <= b.y + b.h + pad;
      const bad = [];
      for (const path of document.querySelectorAll("path.edge")) {
        const [from, to] = path.dataset.edge.split(">");
        const len = path.getTotalLength();
        for (const [q, name] of [[path.getPointAtLength(0), from], [path.getPointAtLength(len), to]]) {
          const own = boxes.find((b) => b.name === name);
          const other = boxes.find((b) => b.name !== name && near(b, q, 1));
          if (!own || !near(own, q, 5) || other) bad.push(`${path.dataset.edge} at ${name}`);
        }
      }
      return bad;
    });
    // Heads: every head is on the crate that depends on the
    // other, an implementation arrow's on the implementer; and no two heads
    // overlap.
    const heads = await p.evaluate(() => {
      const boxOf = (name) => {
        const n = document.querySelector(`.node[aria-label^='${name}:']`);
        const m = /translate\(([-\d.]+),([-\d.]+)\)/.exec(n.getAttribute("transform"));
        const r = n.querySelector("rect");
        return { x: +m[1], y: +m[2], w: +r.getAttribute("width"), h: +r.getAttribute("height") };
      };
      const onEdge = (b, q) => q.x >= b.x - 1.5 && q.x <= b.x + b.w + 1.5 && q.y >= b.y - 1.5 && q.y <= b.y + b.h + 1.5 &&
        (Math.abs(q.x - b.x) < 1.5 || Math.abs(q.x - b.x - b.w) < 1.5 || Math.abs(q.y - b.y) < 1.5 || Math.abs(q.y - b.y - b.h) < 1.5);
      const wrong = [];
      const centres = [];
      let implementing = 0;
      for (const path of document.querySelectorAll("path.edge[marker-start]")) {
        const [from] = path.dataset.edge.split(">");
        const tip = path.getPointAtLength(0);
        if (path.dataset.head !== from || !onEdge(boxOf(from), tip)) wrong.push(path.dataset.edge);
        if (path.classList.contains("role-implements")) implementing++;
        // The head reaches about 7 pixels back along the arrow from its tip.
        const back = path.getPointAtLength(Math.min(3.5, path.getTotalLength()));
        centres.push({ edge: path.dataset.edge, x: back.x, y: back.y });
      }
      const overlapping = [];
      for (let i = 0; i < centres.length; i++) {
        for (let j = i + 1; j < centres.length; j++) {
          if (Math.hypot(centres[i].x - centres[j].x, centres[i].y - centres[j].y) < 7) overlapping.push(`${centres[i].edge} / ${centres[j].edge}`);
        }
      }
      const markers = [...document.querySelectorAll("path.edge[marker-end]")].length;
      return { count: centres.length, implementing, markers, wrong, overlapping };
    });
    check("every head is on the crate that depends, an implementation arrow's on the implementer",
      heads.count > 0 && heads.implementing > 0 && heads.markers === 0 && heads.wrong.length === 0, heads.wrong.slice(0, 5));
    check("no two arrow heads overlap", heads.overlapping.length === 0, heads.overlapping.slice(0, 5));
    // Inside a group the arrow points down, from the head crate to each
    // partner.
    const down = await p.evaluate(() => {
      const path = document.querySelector("path.edge[data-edge='kkt-db-sqlite>kkt-db>normal']");
      return path && { head: path.dataset.head, tip: path.getPointAtLength(0).y, tail: path.getPointAtLength(path.getTotalLength()).y };
    });
    check("inside a group the arrow points down to the partner", down && down.head === "kkt-db-sqlite" && down.tip > down.tail, down);
    const stray = await endsOnOwnBoxes();
    check("every arrow ends on its own crates' boxes", stray.length === 0, stray.slice(0, 5));
    // Every crate's name and subtitle fits inside its box.
    const overflow = await p.evaluate(() => [...document.querySelectorAll(".node")].flatMap((n) => {
      const box = n.querySelector("rect").getBBox();
      return [...n.querySelectorAll("text")].filter((x) => {
        const b = x.getBBox();
        return b.x < box.x || b.x + b.width > box.x + box.width - 8;
      }).map((x) => x.textContent);
    }));
    check("every label fits inside its box", overflow.length === 0, overflow);
    // No band is wider than five boxes on any line: a band wraps at five
    // units, and a group's partners stack beneath its head.
    const wide = await p.evaluate(() => {
      const layer = Object.fromEntries(Object.entries(window.KKT_CONCERNS.crates).map(([n, c]) => [n, c.layer]));
      const rows = new Map();
      for (const n of document.querySelectorAll(".node")) {
        const name = n.getAttribute("aria-label").split(":")[0];
        const y = /translate\(([-\d.]+),([-\d.]+)\)/.exec(n.getAttribute("transform"))[2];
        const key = `${layer[name]} at ${y}`;
        rows.set(key, (rows.get(key) || 0) + 1);
      }
      return [...rows].filter(([, count]) => count > 5).map(([key, count]) => `${key}: ${count}`);
    });
    check("no band is wider than five boxes", wide.length === 0, wide);

    // No box is split: every crate is one kind.
    check("no crate's box is split", (await p.$$eval(".node .impl-part", (x) => x.length)) === 0);
    const client = "kkt-agent-client";
    // A "uses" arrow's head is at the user: drawn from the used crate.
    const head = await p.$eval(`path.edge[data-edge='kkt-agent-commands>${client}>normal'][data-head]`,
      (path) => [path.getAttribute("marker-start"), path.getAttribute("marker-end"), path.dataset.head]);
    check("a uses arrow points at the crate that uses",
      head[0] === "url(#head-uses)" && head[1] === null && head[2] === "kkt-agent-commands", head);
    // A ticked implementation shows its delivery line, clear of the
    // highlighted arrows: kashkeshet sits just above kkt-agent-commands,
    // and a line meeting the boxes where the arrows do lay hidden on them.
    const mcp = "kkt-agent-client-mcp";
    await onBox(mcp, ".pick-hit");
    const clear = await p.evaluate(() => {
      const ends = (path) => [path.getPointAtLength(0), path.getPointAtLength(path.getTotalLength())];
      const line = document.querySelector("path.delivery[data-from='kashkeshet'][data-to='kkt-agent-commands']");
      if (!line) return { drawn: false };
      const arrows = [...document.querySelectorAll("path.edge.focus")].flatMap(ends);
      const near = ends(line).filter((q) => arrows.some((r) => Math.hypot(q.x - r.x, q.y - r.y) < 6));
      return { drawn: true, near: near.length };
    });
    check("a ticked implementation's delivery line is drawn clear of the arrows", clear.drawn && clear.near === 0, clear);
    await onBox(mcp, ".pick-hit");

    // A program's delivery lines leave it at points of their own.
    await onBox("kashkeshet", ".pick-hit");
    const shared = await p.evaluate(() => {
      const starts = [...document.querySelectorAll("path.delivery[data-from='kashkeshet']")].map((x) => x.getPointAtLength(0));
      const close = [];
      for (let i = 0; i < starts.length; i++) {
        for (let j = i + 1; j < starts.length; j++) {
          if (Math.hypot(starts[i].x - starts[j].x, starts[i].y - starts[j].y) < 3) close.push([i, j]);
        }
      }
      return { lines: starts.length, close: close.length };
    });
    check("a program's delivery lines leave it at points of their own", shared.lines > 1 && shared.close === 0, shared);
    await onBox("kashkeshet", ".pick-hit");

    // A point on a drawn path, in screen terms, where nothing covers it.
    const spot = (sel) => p.evaluate((s) => {
      const svgEl = document.getElementById("graph");
      for (const h of document.querySelectorAll(s)) {
        const len = h.getTotalLength();
        for (const f of [0.5, 0.4, 0.6, 0.3, 0.7, 0.2, 0.8, 0.15, 0.85, 0.1, 0.9]) {
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

    // An injection's whole route: ticking kkt-watch draws
    // kashkeshet to kkt-agent-commands to kkt-watch, and the crates that
    // supply what kashkeshet injects, with every hop's label saying what it
    // carries, never "passed on" alone.
    const routeOf = async (name) => {
      await onBox(name, ".pick-hit");
      const got = await p.evaluate(() => ({
        lines: [...document.querySelectorAll("path.delivery")].map((x) => `${x.dataset.from}>${x.dataset.to}`),
        hops: [...new Set([...document.querySelectorAll("path.edge.route-hop")].map((x) => x.dataset.edge))],
        labels: Object.fromEntries([...document.querySelectorAll("text.delivery-label")]
          .map((x) => [`${x.dataset.from}>${x.dataset.to}`, x.textContent])),
      }));
      await onBox(name, ".pick-hit");
      return got;
    };
    const watch = await routeOf("kkt-watch");
    check("a ticked receiver's routes run back to the program and the suppliers",
      ["kashkeshet>kkt-agent-commands", "kkt-agent-commands>kkt-watch"].every((l) => watch.lines.includes(l)) &&
      watch.hops.includes("kashkeshet>kkt-agent-client-mcp>normal"), watch);
    check("no hop's label is only \"passed on\"",
      Object.values(watch.labels).every((l) => !/^passed on/.test(l)) && /\(/.test(watch.labels["kkt-agent-commands>kkt-watch"] || ""),
      watch.labels);
    // A crate's own type is never carried out of it as an injection (msg
    // 3694): kkt-server builds kkt-result-format's Formatter itself, and
    // the injection is the clock that goes into it.
    const format = await routeOf("kkt-result-format");
    check("no route leaves kkt-result-format carrying its own type",
      !format.lines.some((l) => l.startsWith("kkt-result-format>")) &&
      !Object.values(format.labels).some((l) => /Formatter/.test(l)) &&
      format.lines.includes("kkt-server>kkt-result-format"), format);
    const forward = await routeOf("kashkeshet");
    check("a ticked injector's routes run on to the final receiver", forward.lines.includes("kkt-agent-commands>kkt-watch"), forward.lines);
    const supplier = await routeOf("kkt-agent-client-mcp");
    check("a ticked supplier's routes run on to the final receiver",
      supplier.hops.includes("kashkeshet>kkt-agent-client-mcp>normal") && supplier.lines.includes("kkt-agent-commands>kkt-watch"), supplier);

    // Opens a panel section and waits for its body, which a closed section
    // draws only when it opens.
    const openSection = async (id) => {
      await p.evaluate((id) => {
        const d = document.querySelector(`#panel details[data-section='${id}']`);
        if (d && !d.open) d.querySelector("summary").click();
      }, id);
      await p.waitForFunction((id) => {
        const d = document.querySelector(`#panel details[data-section='${id}']`);
        return !d || d.children.length > 1;
      }, id);
    };
    // A value a contract's operation gives: the program asks OsMachine for
    // Env and Platform and hands them down; kkt-install-commands passes them on.
    // Ticking kkt-machine-os lights the program's arrow to it and draws the
    // dotted lines on, labelled with the value and what produced it; the
    // panels say where the values go and where a receiver's come from.
    const machine = await routeOf("kkt-machine-os");
    await onBox("kkt-install-commands", "> rect");
    await openSection("slots");
    const operatePanel = await p.$eval("#panel", (x) => x.textContent);
    await onBox("kkt-machine-os", "> rect");
    await openSection("goes");
    const machinePanel = await p.$eval("#panel", (x) => x.textContent);
    check("Env and Platform are traced from OsMachine through the program to the crates that take them",
      ["kashkeshet>kkt-install-commands", "kashkeshet>kkt-setup", "kashkeshet>kkt-agent-commands", "kkt-install-commands>kkt-setup",
        "kkt-agent-commands>kkt-llm-agent-config"].every((l) => machine.lines.includes(l)) &&
      machine.hops.includes("kashkeshet>kkt-machine-os>normal") &&
      // The line into kkt-install-commands also carries ClientHost and
      // NativeSetupFiles, so its label is shortened; the line into
      // kkt-setup carries only the values.
      /Env, from OsMachine/.test(machine.labels["kashkeshet>kkt-setup"] || "") &&
      /Env, from OsMachine.* · passed on$/.test(machine.labels["kkt-agent-commands>kkt-llm-agent-config"] || "") &&
      /filled with Env, from OsMachine, by kashkeshet/.test(operatePanel) &&
      /Env, from OsMachine → kkt-install-commands's/.test(machinePanel) &&
      // The supply label names each supplied type once.
      machine.labels["kkt-machine-os>kashkeshet"] === "OsMachine; also wrapped by ClientHost, ThisProgram and NativeSetupFiles" &&
      // An adapter over two crates: ClientHost wraps kkt-files' Files and
      // kkt-machine-os' OsMachine, so its route is drawn whole from either,
      // and kkt-files' hop names what wraps its Files.
      /ClientHost \(over kkt-files and kkt-machine-os\)/.test(machine.labels["kashkeshet>kkt-agent-commands"] || "") &&
      machine.hops.includes("kashkeshet>kkt-files>normal") &&
      machine.labels["kkt-files>kashkeshet"] === "Files, wrapped by ClientHost and NativeSetupFiles",
      { machine, operate: (operatePanel.match(/filled with Env[^]{0,60}/) || [""])[0], goes: (machinePanel.match(/Where its work goes[^]{0,120}/) || [""])[0] });
    // ConnectionReads wraps kkt-files, kkt-keys-os and kkt-entropy-os, so
    // ticking kkt-keys-os draws its route into both assemblies.
    const keys = await routeOf("kkt-keys-os");
    check("ticking one of an adapter's several suppliers draws its route",
      ["kashkeshet>kkt-agent-commands", "kashkeshet>kkt-install-commands"].every((l) => keys.lines.includes(l)) &&
      keys.hops.includes("kashkeshet>kkt-keys-os>normal") &&
      /OsKeyFiles.* wrapped by .*ConnectionReads/.test(keys.labels["kkt-keys-os>kashkeshet"] || ""), keys);
    // A value a composition crate is given and passes on is drawn from the
    // program, through it, to each crate that takes it, its own adapters
    // included: KeyMode goes from the program into kkt-server, which passes
    // it to kkt-mcp and wraps it in the web gate it hands kkt-web.
    const identity = await routeOf("kkt-identity-keymode");
    const panelSection = async (name, id) => {
      await onBox(name, "> rect");
      await openSection(id);
      return p.$eval(`#panel details[data-section='${id}']`, (d) => d.textContent);
    };
    const mcpSlots = await panelSection("kkt-mcp", "slots");
    const serverPasses = await panelSection("kkt-server", "passes");
    const keyFilesGo = await panelSection("kkt-keys-os", "goes");
    check("a passed-on value is drawn and named from the program to every crate that takes it",
      ["kashkeshet>kkt-server", "kkt-server>kkt-mcp", "kkt-server>kkt-web"].every((l) => identity.lines.includes(l)) &&
      identity.labels["kashkeshet>kkt-server"] === "KeyMode (kkt-identity-keymode)" &&
      identity.labels["kkt-server>kkt-mcp"] === "KeyMode (kkt-identity-keymode) · passed on" &&
      identity.labels["kkt-server>kkt-web"] === "AdmissionGate (over kkt-admission), wrapping KeyMode (kkt-identity-keymode)" &&
      /filled with KeyMode \(kkt-identity-keymode\), by kashkeshet, passed on by kkt-server/.test(mcpSlots) &&
      /KeyMode \(kkt-identity-keymode\), from kashkeshet → kkt-mcp's endpoint \(identify\)/.test(serverPasses) &&
      /wrapping KeyMode \(kkt-identity-keymode\), from kashkeshet → kkt-web's router \(gate\)/.test(serverPasses) &&
      /OsKeyFiles \(kkt-keys-os\) → kkt-setup's [^;]*by kashkeshet, passed on by kkt-operator-commands/.test(keyFilesGo),
      { identity, mcp: (mcpSlots.match(/endpoint takes identify[^]{0,120}/) || [""])[0], passes: serverPasses.slice(0, 300) });
    // With kkt-server and kkt-identity-keymode both ticked, the line into
    // kkt-server names KeyMode first, so its shortened label keeps it.
    await onBox("kkt-server", ".pick-hit");
    await onBox("kkt-identity-keymode", ".pick-hit");
    const intoServer = await p.$eval("text.delivery-label[data-from='kashkeshet'][data-to='kkt-server']", (x) => x.textContent);
    await onBox("kkt-identity-keymode", ".pick-hit");
    await onBox("kkt-server", ".pick-hit");
    check("a shortened label names the ticked crate's value first", /^KeyMode \(kkt-identity-keymode\)/.test(intoServer), intoServer);
    // Nothing is drawn out of kkt-shutdown-os: the program builds the
    // Shutdown it stops, and StopSignals gives back only which signal came.
    const shutdown = await routeOf("kkt-shutdown-os");
    check("ticking kkt-shutdown-os draws no dotted line", shutdown.lines.length === 0, shutdown);
    // A value asked of an implementer the injector was given: kkt-install-commands
    // asks the clock the program gave it for the time a backup is named for.
    const clock = await routeOf("kkt-clock-system");
    check("the time is traced from SystemClock through kkt-install-commands to kkt-backup",
      clock.lines.includes("kkt-install-commands>kkt-backup") && /Time, from SystemClock/.test(clock.labels["kkt-install-commands>kkt-backup"] || ""),
      clock.labels);

    // A logic crate handing on what it was given: the program gives
    // kkt-rules OsEntropy at start-up, and kkt-rules hands it to kkt-names
    // on each call that draws a handle or a channel ID. Ticking kkt-names
    // draws the route whole, its last hop labelled as passed on.
    const names = await routeOf("kkt-names");
    check("kkt-names' entropy is traced from the program through kkt-rules, and passed on",
      ["kashkeshet>kkt-rules", "kkt-rules>kkt-names"].every((l) => names.lines.includes(l)) &&
      names.hops.includes("kashkeshet>kkt-entropy-os>normal") &&
      /OsEntropy \(kkt-entropy-os\).* · passed on$/.test(names.labels["kkt-rules>kkt-names"] || ""),
      names);
    // The label says a value is passed on, never when: kkt-server hands its
    // parts to kkt-mcp once, when it builds the server, and the code's names
    // cannot tell that from a call that hands it on each time.
    const server = await routeOf("kkt-mcp");
    check("a hop handed on at start-up is not labelled per call",
      / · passed on$/.test(server.labels["kkt-server>kkt-mcp"] || "") &&
      Object.values(server.labels).concat(Object.values(names.labels)).every((l) => !/per call|at start-up/.test(l)), server.labels);

    // An injection through a slot typed as a standard-library trait: the
    // program builds kkt-rolling-file's RotatingFile and hands it to
    // kkt-log-tracing's init_logging as its log file, a std::io::Write.
    // Ticking kkt-rolling-file lights its arrow to the program as the
    // built-by hop and draws the dotted line on to kkt-log-tracing; the
    // panels name the slot, what fills it, and where the work goes.
    const rolling = await routeOf("kkt-rolling-file");
    await onBox("kkt-rolling-file", ".pick-hit");
    const rollingHop = await p.$$eval("path.edge.route-hop[data-edge='kashkeshet>kkt-rolling-file>normal']",
      (xs) => xs.map((x) => x.getAttribute("class").match(/role-\w+/)[0]));
    await onBox("kkt-rolling-file", ".pick-hit");
    await onBox("kkt-log-tracing", "> rect");
    await openSection("slots");
    const tracingPanel = await p.$eval("#panel", (x) => x.textContent);
    await onBox("kkt-rolling-file", "> rect");
    await openSection("goes");
    const rollingPanel = await p.$eval("#panel", (x) => x.textContent);
    check("a log file handed in as a std::io::Write is drawn and named as an injection",
      rolling.lines.includes("kashkeshet>kkt-log-tracing") && /RotatingFile \(kkt-rolling-file\)/.test(rolling.labels["kashkeshet>kkt-log-tracing"] || "") &&
      rollingHop.join() === "role-wires" &&
      /init_logging takes log_file: std::io::Writefilled with RotatingFile \(kkt-rolling-file\), by kashkeshet/.test(tracingPanel) &&
      /RotatingFile \(kkt-rolling-file\) → kkt-log-tracing's init_logging \(log_file\), by kashkeshet/.test(rollingPanel),
      { rolling, rollingHop, tracing: (tracingPanel.match(/init_logging[^]{0,160}/) || [""])[0], goes: (rollingPanel.match(/Where its work goes[^]{0,200}/) || [""])[0] });

    // Solid is a dependency, dotted an injection: ticking
    // kkt-log-tracing lights the solid arrow from it to kashkeshet as the
    // route's first hop, draws no dotted line from it, and every dotted
    // line starts at an injector.
    await onBox("kkt-log-tracing", ".pick-hit");
    const solid = await p.evaluate(() => {
      const injectors = new Set([...Object.keys(window.KKT_ASSEMBLY.programs), ...Object.keys(window.KKT_ASSEMBLY.injectors || {})]);
      const strokes = [...document.querySelectorAll("path.edge[data-edge='kashkeshet>kkt-log-tracing>normal']")];
      return {
        // Only the "built by" stroke is the hop: exactly one
        // amber stroke runs between them, solid, and the others keep their
        // role's colour.
        lit: strokes.filter((x) => x.classList.contains("route-hop")).length === 1 &&
          strokes.some((x) => x.classList.contains("route-hop") && x.classList.contains("role-wires") && getComputedStyle(x).strokeDasharray === "none") &&
          strokes.filter((x) => getComputedStyle(x).stroke === getComputedStyle(strokes.find((y) => y.classList.contains("role-wires"))).stroke).length === 1,
        // No dotted line leaves the supplier: its hop is the solid arrow.
        fromIt: [...document.querySelectorAll("path.delivery")].filter((x) => x.dataset.from === "kkt-log-tracing").length,
        notInjector: [...document.querySelectorAll("path.delivery")].map((x) => x.dataset.from).filter((f) => !injectors.has(f)),
        label: [...document.querySelectorAll("text.delivery-label.supply")].map((x) => x.textContent),
      };
    });
    await onBox("kkt-log-tracing", ".pick-hit");
    check("the supplier's hop is the arrow's one built-by stroke, solid, lit and labelled, and every dotted line starts at an injector",
      solid.lit && solid.fromIt === 0 && solid.notInjector.length === 0 && solid.label.some((l) => /TracingLog/.test(l)), solid);

    // What a crate builds only for its tests is on its test arrow, never a
    // role of its code arrow: kkt-web builds kkt-log's
    // CapturedLog only behind `testing`.
    await onBox("kkt-log", ".pick-hit");
    const builtInCode = () => p.$$eval("path.edge[data-edge='kkt-web>kkt-log>normal'].role-wires", (x) => x.length);
    const testArrow = () => p.$$eval("path.edge[data-edge='kkt-web>kkt-log>dev']", (x) => x.length);
    const codeOnly = { wires: await builtInCode(), tests: await testArrow() };
    await p.click("#arrows-button");
    await p.click("#show-dev");
    const withTests = { wires: await builtInCode(), tests: await testArrow() };
    await p.click("#show-dev");
    await p.keyboard.press("Escape");
    await onBox("kkt-log", ".pick-hit");
    check("a test-only build is on the test arrow, not a built-by stroke of the code arrow",
      codeOnly.wires === 0 && codeOnly.tests === 0 && withTests.wires === 0 && withTests.tests > 0, { codeOnly, withTests });

    // A route is amber from its first hop on: exactly one
    // stroke of the supplier's arrow lights, amber. kkt-terminal's arrow
    // to kashkeshet only uses (WatchOutput wraps its Terminal), so its
    // used-by stroke is the hop; kkt-log-tracing's has a built-by stroke,
    // which is the hop, and its used-by stroke is not lit amber.
    const hopOn = (edge) => p.evaluate(([edge]) => {
      const amber = getComputedStyle(document.querySelector("path.edge.role-wires")).stroke;
      const strokes = [...document.querySelectorAll(`path.edge[data-edge='${edge}']`)];
      return {
        hops: strokes.filter((x) => x.classList.contains("route-hop")).map((x) => x.getAttribute("class").match(/role-\w+/)[0]),
        amber: strokes.filter((x) => getComputedStyle(x).stroke === amber).map((x) => x.getAttribute("class").match(/role-\w+/)[0]),
        labels: [...document.querySelectorAll("text.delivery-label.supply")].map((x) => x.textContent),
      };
    }, [edge]);
    await onBox("kkt-terminal", ".pick-hit");
    const terminal = await hopOn("kashkeshet>kkt-terminal>normal");
    await onBox("kkt-terminal", ".pick-hit");
    check("an adapter's hop is its used-by stroke, lit amber and labelled with what it wraps",
      terminal.hops.join() === "role-uses" && terminal.amber.join() === "role-uses" &&
      terminal.labels.some((l) => /Terminal, wrapped by .*WatchOutput/.test(l)), terminal);
    await onBox("kkt-log-tracing", ".pick-hit");
    const tracing = await hopOn("kashkeshet>kkt-log-tracing>normal");
    await onBox("kkt-log-tracing", ".pick-hit");
    check("a built-by hop lights one amber stroke only, and its used-by stroke is not amber",
      tracing.hops.join() === "role-wires" && tracing.amber.join() === "role-wires", tracing);
    // A hop whose stroke a filter hides is not drawn.
    await onBox("kkt-terminal", ".pick-hit");
    await p.click("#arrows-button");
    await p.click('#role-filters input[data-key="role:uses"]');
    const filtered = await hopOn("kashkeshet>kkt-terminal>normal");
    await p.click('#role-filters input[data-key="role:uses"]');
    await p.keyboard.press("Escape");
    await onBox("kkt-terminal", ".pick-hit");
    check("a hop whose stroke is filtered out is not drawn",
      filtered.hops.length === 0 && !filtered.labels.some((l) => /Terminal, wrapped by/.test(l)), filtered);
    // With nothing ticked, no used-by stroke is amber.
    const resting = await p.evaluate(() => {
      const amber = getComputedStyle(document.querySelector("path.edge.role-wires")).stroke;
      return [...document.querySelectorAll("path.edge.role-uses")].filter((x) => getComputedStyle(x).stroke === amber).length;
    });
    check("with nothing ticked, no used-by stroke is amber", resting === 0, resting);

    // An adapter is not an implementation: a composition's own
    // type meeting another crate's trait is "adapted by", drawn apart from
    // the green arrows; its tooltip says whether it is an adapter or the
    // injector's own work.
    const adapts = await p.evaluate(() => {
      const kind = (n) => (window.KKT_CONCERNS.crates[n] || {}).kind;
      const edges = [...document.querySelectorAll("path.edge")];
      return {
        greenFromComposition: edges.filter((x) => x.classList.contains("role-implements") && kind(x.dataset.edge.split(">")[0]) === "composition")
          .map((x) => x.dataset.edge),
        watch: edges.some((x) => x.dataset.edge === "kashkeshet>kkt-watch>normal" && x.classList.contains("role-adapts")),
        sqlite: edges.some((x) => x.dataset.edge === "kkt-db-sqlite>kkt-db>normal" && x.classList.contains("role-implements")),
      };
    });
    check("no composition's arrow is green, kashkeshet adapts for kkt-watch, kkt-db-sqlite implements kkt-db",
      adapts.greenFromComposition.length === 0 && adapts.watch && adapts.sqlite, adapts);
    const tipOf = async (edge, tick) => {
      await onBox(tick, ".pick-hit");
      // On a narrow page the arrow may run below the fold; bring its middle
      // into view, so a point on it can be hovered.
      await p.evaluate((e) => {
        const h = document.querySelector(`path.edge-hit[data-edge='${e}']`);
        if (h) h.scrollIntoView({ block: "center", inline: "center" });
      }, edge);
      const at = await spot(`path.edge-hit[data-edge='${edge}']`);
      if (at) await p.mouse.move(at.x, at.y);
      const text = at && await p.isVisible("#tip") ? await p.textContent("#tip") : "";
      await p.mouse.move(5, 5);
      await onBox(tick, ".pick-hit");
      return text;
    };
    // Its colour is its own: apart from the injection lines' amber and the
    // "built by" arrows'.
    await onBox("kkt-watch", ".pick-hit");
    const colours = await p.evaluate(() => {
      const style = (sel) => { const x = document.querySelector(sel); return x && getComputedStyle(x); };
      const [a, w, l] = [style("path.edge.role-adapts"), style("path.edge.role-wires"), style("path.delivery")];
      return { adapts: a && a.stroke, wires: w && w.stroke, lines: l && l.stroke,
               adaptsDash: a && a.strokeDasharray, linesDash: l && l.strokeDasharray };
    });
    check("an adapting arrow's colour differs from the injection lines' and the built-by arrows', and its dash from the lines'",
      colours.adapts && colours.wires && colours.lines && colours.adapts !== colours.wires && colours.adapts !== colours.lines &&
      colours.adaptsDash !== colours.linesDash, colours);
    // A route's label covers no box, sits in no gap next to one and in no
    // group's outline, unless it has a backing to read on.
    const covered = await p.evaluate(() => {
      // Each box with a few pixels around it, and each group's outline: a
      // label in the narrow gap beside a box reads as the box's.
      const grow = (r, d) => ({ left: r.left - d, right: r.right + d, top: r.top - d, bottom: r.bottom + d });
      const boxes = [...document.querySelectorAll(".node > rect")].map((r) => grow(r.getBoundingClientRect(), 3))
        .concat([...document.querySelectorAll("rect.group")].map((r) => r.getBoundingClientRect()));
      const backs = [...document.querySelectorAll("rect.delivery-label-back")].map((r) => r.getBoundingClientRect());
      const hit = (a, b) => a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;
      return [...document.querySelectorAll("text.delivery-label")].filter((l) => {
        const r = l.getBoundingClientRect();
        return boxes.some((b) => hit(r, b)) && !backs.some((b) => b.left <= r.left + 1 && b.right >= r.right - 1);
      }).map((l) => l.textContent);
    });
    check("no route label sits bare on a box", covered.length === 0, covered);
    await onBox("kkt-watch", ".pick-hit");
    const ownTip = await tipOf("kkt-install-commands>kkt-setup>normal", "kkt-install-commands");
    check("an adapting arrow names the injector's own work as such", /adapted by: [^]*its own work/.test(ownTip), ownTip.slice(0, 300));
    const gateTip = await tipOf("kkt-server>kkt-web>normal", "kkt-server");
    check("an adapting arrow names the crate its adapter is over", /adapter over kkt-admission/.test(gateTip), gateTip.slice(0, 300));

    // Injections follow the rule: the program injects nothing of
    // kkt-diagnostics' own into it; kkt-install-commands passes on the prober it was
    // given; and the program's own work is labelled as such.
    await onBox("kkt-diagnostics", ".pick-hit");
    const lines = await p.$$eval("path.delivery", (paths) => paths.map((x) => `${x.dataset.from}>${x.dataset.to}`));
    check("no injection of a crate's own type, and the assembly passes on what it was given",
      !lines.includes("kashkeshet>kkt-diagnostics") && lines.includes("kkt-install-commands>kkt-diagnostics"), lines);
    await onBox("kkt-diagnostics", ".pick-hit");
    await onBox("kkt-install-commands", ".pick-hit");
    const labels = await p.$$eval("text.delivery-label", (xs) => xs.map((x) => x.textContent));
    check("the program's own work is named as such", labels.some((l) => l.includes("its own work")), labels);
    await onBox("kkt-install-commands", ".pick-hit");

    // The panel: no "Must not" and no notes on where things come
    // from; each list is a section with a count that opens and closes,
    // short ones open and long ones closed, and the viewer's choice kept.
    await onBox("kkt-watch", "> rect");
    const quiet = await p.$eval("#panel", (x) => x.textContent);
    check("the panel says no \"Must not\" and nothing on where things come from",
      !/Must not/.test(quiet) && !/generated from|hand-written|Cargo's/.test(quiet), quiet.slice(0, 200));
    const sectionState = () => p.$$eval("#panel details.section", (xs) => Object.fromEntries(xs.map((x) => [x.dataset.section, x.open])));
    const before = await sectionState();
    check("sections have counts; short ones start open", before.includes === true &&
      (await p.$$eval("#panel details.section .count", (xs) => xs.every((x) => /^\(\d+\)$/.test(x.textContent)))), before);
    await p.click("#panel details[data-section='includes'] > summary");
    await onBox("kkt-hosts", "> rect");
    await onBox("kkt-watch", "> rect");
    check("a closed section stays closed for the next crate", (await sectionState()).includes === false, await sectionState());
    await p.click("#panel details[data-section='includes'] > summary");
    check("and opens again", (await sectionState()).includes === true);
    await onBox("kashkeshet", "> rect");
    const long = await sectionState();
    // Drawing a section open is not the viewer's choice: with nothing
    // stored, a short list drawn open for one crate leaves nothing behind,
    // and the same section starts closed for a crate whose list is long.
    await p.evaluate(() => localStorage.clear());
    await onBox("kkt-log-tracing", "> rect");
    await onBox("kashkeshet", "> rect");
    const untouched = await p.evaluate(() => ({
      stored: Object.keys(localStorage).filter((k) => k.startsWith("kkt-map-section:")),
      implements: document.querySelector("#panel details[data-section='implements']").open,
    }));
    check("drawing a section open stores no choice", untouched.stored.length === 0 && untouched.implements === false, untouched);
    // Sections the viewer has not touched: kashkeshet implements dozens.
    check("a long list starts closed", long.implements === false && long.items === false, long);

    // Clicking a box shows it in the panel and highlights nothing.
    await onBox("kkt-hosts", "> rect");
    const panel = (await read()).panel;
    check("click shows the panel, no highlight",
      panel === "kkt-hosts" && (await p.$$eval("path.edge.focus", (x) => x.length)) === 0);

    // Ticking highlights and leaves the panel alone.
    await onBox("kkt-watch", ".pick-hit");
    await onBox("kkt-server", ".pick-hit");
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
    // A group whose head is filtered out keeps its outline around the
    // partners still drawn.
    const contract = "#kind-filters input[data-key='kind:contract']";
    await p.click(contract);
    check("a group keeps its outline when its head is hidden",
      (await p.$$eval("rect.group[data-group='kkt-db']", (x) => x.length)) === 1 &&
      (await p.$$eval(node("kkt-db"), (x) => x.length)) === 0, null);
    await unmoved("a group's head hidden", home);
    await p.click(contract);
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
    await onBox("kashkeshet", ".eye-hit");
    await same("a program hidden", both, panel);
    await unmoved("a program hidden", home);
    await onBox("kashkeshet", ".eye-hit");
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
