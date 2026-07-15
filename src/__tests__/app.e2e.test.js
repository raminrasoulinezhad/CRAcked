// End-to-end frontend test. This wires the *real* index.html markup into jsdom,
// mocks the Tauri IPC bridge (`window.__TAURI__.core.invoke`) with an in-memory
// fake backend, then drives `init()` and real user interactions (form submits,
// tab switches) and asserts on the resulting DOM — the same path a user hits,
// minus the WebView. It exercises the whole frontend: event wiring, invoke
// payload shaping (dollars → cents), and render logic.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { beforeEach, afterEach, describe, it, expect, vi } from "vitest";

// index.html script tag lives in <head>; the <body> is pure app markup.
// Read via the filesystem (vitest runs from the project root) rather than a
// URL — under jsdom `import.meta.url` is not a file:// URL.
const html = readFileSync(join(process.cwd(), "src", "index.html"), "utf8");
const bodyHtml = html
  .match(/<body[^>]*>([\s\S]*?)<\/body>/i)[1]
  .replace(/<script[\s\S]*?<\/script>/gi, "");

const $ = (sel) => document.querySelector(sel);

/** A YearComputation-shaped object (snake_case, matching the Rust serialize). */
function rrspYear(over) {
  return {
    year: 2024,
    new_room: 9_000_00,
    opening_room: 0,
    available_room: 9_000_00,
    contribution: over ? 12_000_00 : 4_000_00,
    closing_room: over ? -3_000_00 : 5_000_00,
    over_contribution: over ? 1_000_00 : 0,
    estimated_monthly_penalty: over ? 10_00 : 0,
    dollar_limit_missing: false,
  };
}

function rrspSummary({ over = false } = {}) {
  const y = rrspYear(over);
  return {
    years: [y],
    current_room: y.closing_room,
    total_contributed: y.contribution,
    current_over_contribution: y.over_contribution,
    opening_room: 0,
    missing_limit_years: [],
    latest_built_in_year: 2026,
    projection: null,
  };
}

let backend; // command name -> handler(args)
let invokeSpy;

/** Rebuild a fresh fake backend + DOM before each test. */
beforeEach(async () => {
  vi.resetModules();
  document.body.innerHTML = bodyHtml;

  const state = { over: false, income: [], contribs: [] };

  backend = {
    list_persons: () => [{ id: 1, name: "Me" }],
    get_backup_settings: () => ({
      remote: "",
      folder: "CRAcked",
      dir: "/data/CRAcked",
      enabled: false,
    }),
    get_rrsp_summary: () => rrspSummary({ over: state.over }),
    list_annual_income: () => state.income,
    list_contributions: () => state.contribs,
    get_tfsa_settings: () => ({ start_year: null, opening_room: 0 }),
    get_fhsa_settings: () => ({ open_year: null }),
    upsert_annual_income: (args) => {
      state.income = [
        {
          year: args.year,
          earned_income_cents: args.earnedIncomeCents,
          pension_adjustment_cents: args.pensionAdjustmentCents,
          is_estimate: args.isEstimate,
        },
      ];
    },
  };

  invokeSpy = vi.fn((cmd, args) => {
    const handler = backend[cmd];
    return Promise.resolve(handler ? handler(args) : undefined);
  });
  window.__TAURI__ = { core: { invoke: invokeSpy } };

  // Expose a switch so a test can turn on the over-contribution scenario.
  window.__setOver = (v) => {
    state.over = v;
  };
});

afterEach(() => {
  delete window.__TAURI__;
  delete window.__setOver;
});

async function boot() {
  const app = await import("../main.js");
  await app.init();
  // init() fires several async refreshes; let microtasks settle.
  await Promise.resolve();
  await Promise.resolve();
  return app;
}

describe("startup", () => {
  it("renders the RRSP tab with room and a year row", async () => {
    const app = await boot();

    expect(invokeSpy).toHaveBeenCalledWith("list_persons");
    // RRSP tab is the default active view.
    expect($(".tab.active").dataset.account).toBe("RRSP");
    expect($('.account-view[data-view="RRSP"]').hidden).toBe(false);

    // The room card shows the formatted current room, and the year table has a row.
    expect($("#rrsp-room").textContent).toBe(app.fmt(5_000_00));
    const rows = document.querySelectorAll("#rrsp-year-body tr");
    expect(rows.length).toBe(1);
    expect(rows[0].textContent).toContain("2024");
    // No over-contribution → the warning card stays hidden.
    expect($("#rrsp-over-card").hidden).toBe(true);
  });

  it("shows the over-contribution card and penalty when over the limit", async () => {
    window.__setOver(true);
    const app = await boot();

    expect($("#rrsp-over-card").hidden).toBe(false);
    expect($("#rrsp-over").textContent).toBe(app.fmt(1_000_00));
    expect($("#rrsp-room").classList.contains("warn")).toBe(true);
    expect($("#rrsp-warnings").textContent).toContain("penalty");
  });
});

describe("adding RRSP income", () => {
  it("submits dollars as integer cents and re-renders", async () => {
    await boot();
    invokeSpy.mockClear();

    $("#rrsp-income-year").value = "2023";
    $("#rrsp-income-amount").value = "50000";
    $("#rrsp-income-pa").value = "";
    $("#rrsp-income-estimate").checked = false;

    $("#rrsp-income-form").dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    // Let the async submit handler run.
    await new Promise((r) => setTimeout(r, 0));

    const call = invokeSpy.mock.calls.find((c) => c[0] === "upsert_annual_income");
    expect(call).toBeTruthy();
    expect(call[1]).toMatchObject({
      year: 2023,
      earnedIncomeCents: 5_000_000, // $50,000 → cents
      pensionAdjustmentCents: 0,
      isEstimate: false,
    });

    // The income now shows in the list, and the summary was refreshed.
    expect(invokeSpy.mock.calls.some((c) => c[0] === "get_rrsp_summary")).toBe(true);
    expect($("#rrsp-income-list").textContent).toContain("2023");
    expect($("#rrsp-income-list").textContent).toContain("50,000");
  });
});

describe("tab switching", () => {
  it("shows the TFSA setup prompt when TFSA is unconfigured", async () => {
    const app = await boot();
    app.switchTo("TFSA");
    await new Promise((r) => setTimeout(r, 0));

    expect($('.account-view[data-view="TFSA"]').hidden).toBe(false);
    expect($("#tfsa-setup").hidden).toBe(false); // not configured → setup shown
    expect($("#tfsa-body").hidden).toBe(true);
  });
});
