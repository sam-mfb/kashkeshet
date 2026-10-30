// Checks the toasts and front-page dashboard updates for leaving, hiding
// channels, and unregistering or hiding agents, in jsdom, against a running
// web preview. Not run by `cargo test`: it needs Node and jsdom.
//
//   cargo run --example web_preview -- --bind 127.0.0.1:7499 --seed 7
//   cd tests/web_script && npm ci && node toasts.mjs http://127.0.0.1:7499
//
// The events are delivered by hand, so the preview's own feed does not
// matter; only its first page is used.
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

const base = process.argv[2];
if (!base) throw new Error("usage: node toasts.mjs <preview base URL>");

/// The front page at `path` in jsdom, with its script running.
/// `send(name, data)` delivers a live event.
async function page(path) {
  const html = await (await fetch(`${base}${path}`)).text();
  const listeners = {};
  const dom = new JSDOM(html, {
    url: `${base}${path}`,
    runScripts: "dangerously",
    beforeParse(window) {
      window.TextEncoder = TextEncoder;
      window.EventSource = class {
        addEventListener(name, fn) { listeners[name] = fn; }
        close() {}
      };
      window.scrollTo = () => {};
      window.scrollBy = () => {};
    },
  });
  const { document } = dom.window;
  return {
    document,
    send: (name, data) => listeners[name]({ data: JSON.stringify(data) }),
    lastToast: () => [...document.querySelectorAll("#toasts .toast")].at(-1)?.textContent,
    channelRow: (id) => document.querySelector(`table[data-dashboard] tr[data-channel-id="${id}"]`),
    members: (id) => document.querySelector(`tr[data-channel-id="${id}"] td.members`)?.textContent,
    agentRow: (name) => document.querySelector(`table.agents tr[data-agent="${name}"]`),
    channelsOf: (name) => [...document.querySelectorAll(`tr[data-agent="${name}"] td.channels a[data-channel-id]`)]
      .map((a) => a.dataset.channelId),
  };
}

/// An agent on the page that is in a channel, and that channel.
function member(p) {
  for (const row of p.document.querySelectorAll("table.agents tr[data-agent]")) {
    const link = row.querySelector("td.channels a[data-channel-id]");
    if (link) return { agent: row.dataset.agent, id: link.dataset.channelId, name: link.textContent.slice(1) };
  }
  throw new Error("no agent in a channel");
}

// Leaving: by itself, or removed by the owner or the operator.
{
  const p = await page("/");
  const { agent, id, name } = member(p);
  p.send("left", { agent, channel_id: id, channel_name: name, member_count: 7, by: agent, by_operator: false });
  assert.equal(p.lastToast(), `←${agent} left #${name}`);
  assert.equal(p.members(id), "7");
  assert.ok(!p.channelsOf(agent).includes(id), "the agent no longer lists the channel");

  p.send("left", { agent, channel_id: id, channel_name: name, member_count: 6, by: "owner-agent", by_operator: false });
  assert.equal(p.lastToast(), `←owner-agent removed ${agent} from #${name}`);
  p.send("left", { agent, channel_id: id, channel_name: name, member_count: 5, by: "operator", by_operator: true });
  assert.equal(p.lastToast(), `←operator removed ${agent} from #${name}`);
  assert.equal(p.members(id), "5");
  console.log("ok: left");
}

// Hiding a channel: its row goes, and no agent lists it.
{
  const p = await page("/");
  const { agent, id, name } = member(p);
  p.send("channel_hidden", { channel_id: id, channel_name: name, by: "owner-agent", by_operator: false });
  assert.equal(p.lastToast(), `🚫owner-agent hid #${name}`);
  assert.equal(p.channelRow(id), null, "the row is gone");
  assert.ok(!p.channelsOf(agent).includes(id));
  p.send("channel_hidden", { channel_id: id, channel_name: name, by: "operator", by_operator: true });
  assert.equal(p.lastToast(), `🚫operator hid #${name}`);
  console.log("ok: channel_hidden");
}

// With hidden channels shown, the row stays, marked hidden, with no members.
{
  const p = await page("/?hidden=1");
  const { id, name } = member(p);
  p.send("channel_hidden", { channel_id: id, channel_name: name, by: "operator", by_operator: true });
  const row = p.channelRow(id);
  assert.ok(row, "the row stays");
  assert.ok(row.hasAttribute("data-hidden-channel"));
  assert.ok(row.querySelector(".badge.gone"));
  assert.equal(p.members(id), "0");
  console.log("ok: channel_hidden with hidden channels shown");
}

// Unregistering and hiding an agent: its row goes, and member counts update.
{
  const p = await page("/");
  const { agent, id, name } = member(p);
  const channels = [{ channel_id: id, channel_name: name, member_count: 3 }];
  p.send("agent_hidden", { agent, by: agent, by_operator: false, channels });
  assert.equal(p.lastToast(), `🚪${agent} unregistered`);
  assert.equal(p.agentRow(agent), null, "the row is gone");
  assert.equal(p.members(id), "3");
  p.send("agent_hidden", { agent, by: "operator", by_operator: true, channels: [] });
  assert.equal(p.lastToast(), `🚪operator hid ${agent}`);
  console.log("ok: agent_hidden");
}
