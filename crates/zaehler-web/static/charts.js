// Diagramme der Auswertung. ECharts wird nur auf Seiten mit Diagramm
// nachgeladen; ohne JavaScript bleiben die Tabellen unter den Diagrammen.
"use strict";

// @impl SPEC_ANA_CHARTS
(() => {
  let loading = null;
  const charts = new Set();

  function loadEcharts(src) {
    if (window.echarts) {
      return Promise.resolve(window.echarts);
    }
    if (!loading) {
      loading = new Promise((resolve, reject) => {
        const script = document.createElement("script");
        script.src = src;
        script.onload = () => resolve(window.echarts);
        script.onerror = () => {
          loading = null;
          reject(new Error("ECharts nicht geladen"));
        };
        document.head.appendChild(script);
      });
    }
    return loading;
  }

  function css(name) {
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  }

  // Tooltip als DOM statt HTML-Text: die CSP verbietet Inline-Styles, mit
  // denen ECharts sonst Abstand und Farbmarke setzt (SPEC_SEC_HEADERS).
  function tooltip(params, nf, unit) {
    const box = document.createElement("div");
    const head = document.createElement("div");
    head.className = "chart-tip-head";
    head.textContent = params[0]?.axisValueLabel ?? "";
    box.append(head);
    params.forEach((p) => {
      const row = document.createElement("div");
      row.className = "chart-tip-row";
      const mark = document.createElement("span");
      mark.className = "chart-tip-mark";
      mark.style.background = p.color;
      const name = document.createElement("span");
      name.textContent = p.seriesName;
      const value = document.createElement("span");
      value.className = "chart-tip-value";
      value.textContent = p.value == null ? "-" : `${nf.format(p.value)} ${unit}`;
      row.append(mark, name, value);
      box.append(row);
    });
    return box;
  }

  function base(data) {
    const fg = css("--fg");
    const muted = css("--muted");
    const border = css("--border");
    const nf = new Intl.NumberFormat(document.documentElement.lang || undefined, {
      maximumFractionDigits: 1,
    });
    return {
      animation: !window.matchMedia("(prefers-reduced-motion: reduce)").matches,
      textStyle: { color: fg, fontFamily: getComputedStyle(document.body).fontFamily },
      grid: { left: 8, right: 16, top: 48, bottom: 8, containLabel: true },
      legend: { top: 0, textStyle: { color: fg } },
      tooltip: {
        trigger: "axis",
        formatter: (params) => tooltip(params, nf, data.unit),
      },
      xAxis: {
        type: "category",
        data: data.labels,
        axisLine: { lineStyle: { color: border } },
        axisLabel: { color: muted },
      },
      yAxis: {
        type: "value",
        name: data.unit,
        nameTextStyle: { color: muted },
        axisLabel: { color: muted, formatter: (v) => nf.format(v) },
        splitLine: { lineStyle: { color: border } },
      },
    };
  }

  const builders = {
    // Monatsbalken laufendes Jahr und Vorjahr; unvollständige Monate bleiben leer
    monthly(data) {
      const s = data.series;
      return {
        ...base(data),
        color: [css("--accent"), css("--muted")],
        series: [
          { type: "bar", name: s.current.name, data: s.current.data },
          { type: "bar", name: s.previous.name, data: s.previous.data },
        ],
      };
    },
    // Kumulierter Verlauf, Prognose gestrichelt bis Jahresende
    cumulative(data) {
      const s = data.series;
      const accent = css("--accent");
      return {
        ...base(data),
        color: [accent, accent],
        series: [
          { type: "line", name: s.cumulative.name, data: s.cumulative.data, symbolSize: 6 },
          {
            type: "line",
            name: s.forecast.name,
            data: s.forecast.data,
            symbol: "none",
            lineStyle: { type: "dashed" },
          },
        ],
      };
    },
  };

  // `requests`: ein Abruf je URL für alle Diagrammbereiche einer Seite
  async function render(section, requests) {
    const nodes = section.querySelectorAll(".chart[data-chart]");
    if (nodes.length === 0) {
      return;
    }
    const url = section.dataset.chartUrl;
    if (!requests.has(url)) {
      requests.set(
        url,
        fetch(url, { headers: { Accept: "application/json" } }).then((r) => {
          if (!r.ok) {
            throw new Error(`HTTP ${r.status}`);
          }
          return r.json();
        }),
      );
    }
    try {
      const [echarts, data] = await Promise.all([
        loadEcharts(section.dataset.echarts),
        requests.get(url),
      ]);
      nodes.forEach((node) => {
        const build = builders[node.dataset.chart];
        // `data-register`: Diagramm eines Registers (SPEC_ANA_REGISTER_CHARTS)
        const series =
          node.dataset.register === undefined
            ? data
            : { ...data.registers?.[Number(node.dataset.register)], unit: data.unit };
        if (!build || !series.series) {
          return;
        }
        echarts.getInstanceByDom(node)?.dispose();
        // Inhalt aus einem wiederhergestellten HTMX-Verlauf entfernen
        node.replaceChildren();
        const chart = echarts.init(node);
        chart.setOption(build(series));
        charts.add(chart);
      });
    } catch {
      // Die Tabellen unter den Diagrammen bleiben verfügbar.
      section.classList.add("charts-failed");
    }
  }

  function init() {
    charts.forEach((c) => {
      if (!c.getDom().isConnected) {
        c.dispose();
        charts.delete(c);
      }
    });
    const requests = new Map();
    document.querySelectorAll("[data-chart-url]").forEach((section) => render(section, requests));
  }

  document.addEventListener("DOMContentLoaded", init);
  document.addEventListener("htmx:afterSettle", init);
  document.addEventListener("htmx:historyRestore", init);
  window.addEventListener("resize", () => charts.forEach((c) => c.resize()));
})();
