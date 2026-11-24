// Draws what the chat's HTML only marks (comandos_core::chat_markdown): Mermaid
// diagrams (`.cv-mermaid[data-src]`) and the first page of internet PDFs
// (`.cv-pdf[data-src]`). Shared by the web chat and the desktop's chat page.
// Each library loads once, only when a diagram or a PDF shows up, from cdnjs at a
// pinned version with SRI (the worker too: fetched with its hash, run from a blob).
// A diagram that fails keeps its code; a PDF that cannot be read keeps its link.
(function () {
  if (window.__cvMedia) return;
  var CDN = "https://cdnjs.cloudflare.com/ajax/libs/";
  var LIBS = {
    mermaid: [CDN + "mermaid/10.9.1/mermaid.min.js", "sha384-WmdflGW9aGfoBdHc4rRyWzYuAjEmDwMdGdiPNacbwfGKxBW/SO6guzuQ76qjnSlr"],
    pdf: [CDN + "pdf.js/3.11.174/pdf.min.js", "sha384-/1qUCSGwTur9vjf/z9lmu/eCUYbpOTgSjmpbMQZ1/CtX2v/WcAIKqRv+U1DUCG6e"],
  };
  var WORKER = [CDN + "pdf.js/3.11.174/pdf.worker.min.js", "sha384-SnzOobpRMLXZ52iJvZm/C0fYw0OQemTXzTjIsdsfMcrCtCEe9qgzxTd3RSklO5x2"];
  var loading = {};
  function lib(name) {
    if (!loading[name]) {
      loading[name] = new Promise(function (ok, fail) {
        var s = document.createElement("script");
        s.src = LIBS[name][0];
        s.integrity = LIBS[name][1];
        s.crossOrigin = "anonymous";
        s.referrerPolicy = "no-referrer";
        s.onload = ok;
        s.onerror = function () {
          delete loading[name];
          fail(new Error(name));
        };
        document.head.appendChild(s);
      });
    }
    return loading[name];
  }
  function dark() {
    var c = (getComputedStyle(document.body).backgroundColor.match(/\d+/g) || [0, 0, 0]).map(Number);
    return (c[0] + c[1] + c[2]) / 3 < 128;
  }
  var n = 0;
  var mermaidTheme = "";
  function drawMermaid(el) {
    el.setAttribute("data-cv-done", "1");
    lib("mermaid")
      .then(function () {
        var theme = dark() ? "dark" : "default";
        if (theme !== mermaidTheme) {
          mermaidTheme = theme;
          window.mermaid.initialize({ startOnLoad: false, securityLevel: "strict", theme: theme });
        }
        return window.mermaid.render("cv-mermaid-" + ++n, el.getAttribute("data-src") || "");
      })
      .then(function (r) {
        el.innerHTML = r.svg; // securityLevel strict: mermaid sanitizes its SVG
        el.classList.add("drawn");
        var code = el.nextElementSibling;
        if (code && code.classList.contains("cv-code")) code.classList.add("cv-under-diagram");
      })
      .catch(function () {
        el.remove(); // the code under it stays
      });
  }
  var worker = null;
  function pdfWorker() {
    if (!worker) {
      worker = fetch(WORKER[0], { integrity: WORKER[1], mode: "cors", referrerPolicy: "no-referrer" })
        .then(function (r) {
          if (!r.ok) throw new Error("worker");
          return r.text();
        })
        .then(function (code) {
          window.pdfjsLib.GlobalWorkerOptions.workerSrc = URL.createObjectURL(new Blob([code], { type: "text/javascript" }));
        })
        .catch(function (e) {
          worker = null;
          throw e;
        });
    }
    return worker;
  }
  function drawPdf(el) {
    el.setAttribute("data-cv-done", "1");
    var url = el.getAttribute("data-src") || "";
    lib("pdf")
      .then(pdfWorker)
      .then(function () {
        return window.pdfjsLib.getDocument({ url: url, isEvalSupported: false, disableAutoFetch: true }).promise;
      })
      .then(function (doc) {
        return doc.getPage(1).then(function (page) {
          var width = Math.min(el.parentElement ? el.parentElement.clientWidth : 360, 460) || 360;
          var base = page.getViewport({ scale: 1 });
          var scale = Math.min(3, (width * (window.devicePixelRatio || 1)) / base.width);
          var vp = page.getViewport({ scale: scale });
          var canvas = document.createElement("canvas");
          canvas.width = Math.round(vp.width);
          canvas.height = Math.round(vp.height);
          canvas.className = "cv-pdf-page";
          return page.render({ canvasContext: canvas.getContext("2d"), viewport: vp }).promise.then(function () {
            el.insertBefore(canvas, el.firstChild);
            el.classList.add("drawn");
            if (doc.numPages > 1) {
              var more = document.createElement("small");
              more.textContent = doc.numPages + " páginas";
              el.appendChild(more);
            }
          });
        });
      })
      .catch(function () {
        /* No CORS on that site, or not a PDF: the link stays. */
      });
  }
  window.__cvMedia = function (root) {
    var scope = root && root.querySelectorAll ? root : document;
    Array.prototype.forEach.call(scope.querySelectorAll(".cv-mermaid:not([data-cv-done])"), drawMermaid);
    Array.prototype.forEach.call(scope.querySelectorAll(".cv-pdf:not([data-cv-done])"), drawPdf);
  };
})();
