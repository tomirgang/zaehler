// Diagramme der Auswertung. ECharts wird nur auf Seiten mit Diagramm
// nachgeladen; ohne JavaScript bleiben die Tabellen unter den Diagrammen.
"use strict";

// @impl SPEC_ANA_CHARTS, SPEC_ANA_PARTIAL_MONTHS, SPEC_ANA_CUMULATIVE_ESTIMATE
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
    // Prognose- und Schätzreihen nur in Monaten mit Wert
    params.forEach((p) => {
      if (p.value == null && String(p.seriesId).endsWith("-estimate")) {
        return;
      }
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

  const present = (series) => series?.data?.some((v) => v != null);

  // Schraffur für prognostizierte Anteile (SPEC_ANA_PARTIAL_MONTHS)
  const hatch = {
    symbol: "rect",
    symbolSize: 1,
    dashArrayX: [1, 0],
    dashArrayY: [2, 4],
    rotation: -Math.PI / 4,
    color: "rgba(255, 255, 255, 0.75)",
  };

  const builders = {
    // Monatsbalken laufendes Jahr und Vorjahr; teilweise abgedeckte Monate
    // als gemessener Anteil mit gestapelter, schraffierter Prognose
    // (SPEC_ANA_PARTIAL_MONTHS); nicht abgedeckte Monate bleiben leer
    monthly(data) {
      const s = data.series;
      const colors = { current: css("--accent"), previous: css("--muted") };
      const series = [];
      ["current", "previous"].forEach((key) => {
        const color = colors[key];
        series.push({
          id: key,
          type: "bar",
          stack: key,
          name: s[key].name,
          data: s[key].data,
          itemStyle: { color },
        });
        const projected = s[`${key}_projected`];
        if (present(projected)) {
          series.push({
            id: `${key}-estimate`,
            type: "bar",
            stack: key,
            name: projected.name,
            data: projected.data,
            itemStyle: { color, opacity: 0.55, decal: hatch },
          });
        }
      });
      return { ...base(data), series };
    },
    // Kumulierter Verlauf, geschätzter Jahresbeginn gepunktet
    // (SPEC_ANA_CUMULATIVE_ESTIMATE), Prognose gestrichelt bis Jahresende
    cumulative(data) {
      const s = data.series;
      const accent = css("--accent");
      const series = [
        {
          type: "line",
          name: s.cumulative.name,
          data: s.cumulative.data,
          symbolSize: 6,
          itemStyle: { color: accent },
        },
      ];
      if (present(s.estimate)) {
        series.push({
          id: "cumulative-estimate",
          type: "line",
          name: s.estimate.name,
          data: s.estimate.data,
          symbol: "none",
          itemStyle: { color: accent },
          lineStyle: { type: "dotted", width: 2 },
        });
      }
      series.push({
        id: "forecast-estimate",
        type: "line",
        name: s.forecast.name,
        data: s.forecast.data,
        symbol: "none",
        itemStyle: { color: accent },
        lineStyle: { type: "dashed" },
      });
      return { ...base(data), series };
    },
    // Monatskosten: Bezug gemessen und prognostiziert (schraffiert)
    // gestapelt, Einspeisevergütung als negativer Balken (SPEC_TAR_COST_VIEW)
    costs(data) {
      const s = data.series;
      const accent = css("--accent");
      const series = [
        {
          id: "measured",
          type: "bar",
          stack: "import",
          name: s.measured.name,
          data: s.measured.data,
          itemStyle: { color: accent },
        },
        {
          id: "costs-estimate",
          type: "bar",
          stack: "import",
          name: s.estimated.name,
          data: s.estimated.data,
          itemStyle: { color: accent, opacity: 0.55, decal: hatch },
        },
      ];
      if (present(s.export)) {
        series.push({
          id: "export",
          type: "bar",
          stack: "import",
          name: s.export.name,
          data: s.export.data,
          itemStyle: { color: css("--muted") },
        });
      }
      return { ...base(data), series };
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
