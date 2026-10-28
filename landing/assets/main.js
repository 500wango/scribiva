/* Scribiva landing — applies config.js values and runs the one hero demo. No network calls. */
(function () {
  "use strict";
  var cfg = window.SCRIBIVA_CONFIG || {};
  var lang = (document.documentElement.lang || "zh").slice(0, 2) === "en" ? "en" : "zh";

  function val(key) {
    if (cfg[key + "_" + lang] !== undefined) return cfg[key + "_" + lang];
    return cfg[key];
  }

  // Text placeholders: <span data-cfg="price_monthly">…</span>
  Array.prototype.forEach.call(document.querySelectorAll("[data-cfg]"), function (el) {
    var v = val(el.getAttribute("data-cfg"));
    if (typeof v === "string" && v) el.textContent = v;
  });

  // Contact links: <a data-cfg-mailto>…</a>
  Array.prototype.forEach.call(document.querySelectorAll("[data-cfg-mailto]"), function (el) {
    if (cfg.contact_email) {
      el.href = "mailto:" + cfg.contact_email;
      el.textContent = cfg.contact_email;
    }
  });

  // Downloads: switch from "coming soon" to a real link once a URL is configured.
  if (cfg.release_state === "available") {
    Array.prototype.forEach.call(document.querySelectorAll("[data-dl]"), function (el) {
      var url = cfg[el.getAttribute("data-dl") + "_url"];
      if (!url) return;
      var a = document.createElement("a");
      a.className = "btn";
      a.href = url;
      a.textContent = el.getAttribute("data-ready-label") || el.textContent;
      el.parentNode.replaceChild(a, el);
    });
    Array.prototype.forEach.call(document.querySelectorAll("[data-dl-state]"), function (el) {
      el.hidden = true;
    });
  }

  // Hero demo: one short sequence on load (recording → transcribing → inserted).
  var hud = document.querySelector(".hud");
  var out = document.querySelector("[data-demo-text]");
  var reduce = window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  if (!hud || !out || reduce) return;

  var labels = {
    zh: { rec: "正在录音 · 00:0", work: "正在修正语法…", done: "已插入光标位置" },
    en: { rec: "Recording · 00:0", work: "Correcting grammar…", done: "Inserted at cursor" }
  }[lang];
  var text = out.innerHTML;
  var label = hud.querySelector(".hud-label");
  out.innerHTML = "";
  hud.setAttribute("data-state", "rec");

  var sec = 1;
  label.textContent = labels.rec + sec;
  var tick = setInterval(function () {
    sec += 1;
    label.textContent = labels.rec + sec;
    if (sec >= 4) {
      clearInterval(tick);
      hud.setAttribute("data-state", "work");
      label.textContent = labels.work;
      setTimeout(function () {
        out.innerHTML = text;
        hud.setAttribute("data-state", "done");
        label.textContent = labels.done;
      }, 1100);
    }
  }, 700);
})();
