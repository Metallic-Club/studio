// Sets the page theme before the first paint. Loaded in <head> without defer, so the page never
// shows one theme and then another. The saved choice is kept in this page store and in the
// Launcher's settings file; pages.js copies the settings file's value here when they differ.
(function () {
  "use strict";

  var KEY = "metallic.launcher.theme";
  // The shared token themes (tokens.css has a [data-theme] block for each) plus "system".
  var THEMES = ["graphite", "titanium", "light"];
  var root = document.documentElement;
  var lightQuery = window.matchMedia ? window.matchMedia("(prefers-color-scheme: light)") : null;
  var choice = "graphite";

  function valid(word) {
    return word === "system" || THEMES.indexOf(word) !== -1;
  }

  // System follows the light or dark setting: Light when light, Graphite when dark.
  function resolve(word) {
    if (word !== "system") {
      return word;
    }
    return lightQuery && lightQuery.matches ? "light" : "graphite";
  }

  function apply(word) {
    if (!valid(word)) {
      return;
    }
    choice = word;
    root.setAttribute("data-theme", resolve(word));
  }

  function remember(word) {
    if (!valid(word)) {
      return;
    }
    try {
      window.localStorage.setItem(KEY, word);
    } catch (e) {
      // Storage can be off; the page still follows the settings file once pages.js reads it.
    }
    apply(word);
  }

  var saved = null;
  try {
    saved = window.localStorage.getItem(KEY);
  } catch (e) {
    saved = null;
  }
  apply(valid(saved) ? saved : "graphite");

  if (lightQuery && lightQuery.addEventListener) {
    lightQuery.addEventListener("change", function () {
      if (choice === "system") {
        apply("system");
      }
    });
  }

  window.metallicTheme = {
    // The saved choice: "system", "graphite", "titanium" or "light".
    choice: function () { return choice; },
    // Shows a choice and remembers it for the next page.
    remember: remember
  };
})();
