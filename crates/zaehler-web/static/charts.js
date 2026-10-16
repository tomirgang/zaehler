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
        valueFormatter: (v) => (v == null ? "-" : `${nf.format(v)} ${data.unit}`),
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

  async function render(section) {
    const nodes = section.querySelectorAll(".chart[data-chart]");
    if (nodes.length === 0) {
      return;
    }
    try {
      const [echarts, data] = await Promise.all([
        loadEcharts(section.dataset.echarts),
        fetch(section.dataset.chartUrl, { headers: { Accept: "application/json" } }).then((r) => {
          if (!r.ok) {
            throw new Error(`HTTP ${r.status}`);
          }
          return r.json();
        }),
      ]);
      nodes.forEach((node) => {
        const build = builders[node.dataset.chart];
        if (!build) {
          return;
        }
        echarts.getInstanceByDom(node)?.dispose();
        // Inhalt aus einem wiederhergestellten HTMX-Verlauf entfernen
        node.replaceChildren();
        const chart = echarts.init(node);
        chart.setOption(build(data));
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
    document.querySelectorAll("[data-chart-url]").forEach(render);
  }

  document.addEventListener("DOMContentLoaded", init);
  document.addEventListener("htmx:afterSettle", init);
  document.addEventListener("htmx:historyRestore", init);
  window.addEventListener("resize", () => charts.forEach((c) => c.resize()));
})();
