// Product analytics for the operator console. This stays a same-origin static
// file so the desktop's strict CSP need not permit inline scripts.
window.op = window.op || function () {
  var queue = [];
  return new Proxy(function () {
    if (arguments.length) queue.push(Array.prototype.slice.call(arguments));
  }, {
    get: function (_target, property) {
      return property === "q"
        ? queue
        : function () { queue.push([property].concat(Array.prototype.slice.call(arguments))); };
    },
    has: function (_target, property) { return property === "q"; },
  });
}();

window.op("init", {
  apiUrl: "https://panel.tinyhumans.ai/api",
  clientId: "afe8ec4e-0a6a-427a-aa22-49cbbf137d0a",
  trackScreenViews: true,
  trackOutgoingLinks: true,
  trackAttributes: true,
});

var openPanelScript = document.createElement("script");
openPanelScript.src = "https://openpanel.dev/op1.js";
openPanelScript.async = true;
document.head.appendChild(openPanelScript);
