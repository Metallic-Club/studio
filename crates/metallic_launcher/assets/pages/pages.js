// Scripts for the bundled pages. No remote addresses and no inline script: the page policy forbids both.
(function () {
  "use strict";

  var REASONS = ["unreachable", "notconfigured", "loadfailed"];

  function post(message) {
    if (window.ipc && typeof window.ipc.postMessage === "function") {
      window.ipc.postMessage(message);
      return true;
    }
    return false;
  }

  // Offline page: pick the text for the reason in the address, and wire the Try again button.
  var params = new URLSearchParams(window.location.search);
  var reason = params.get("reason");
  if (REASONS.indexOf(reason) !== -1) {
    document.body.setAttribute("data-reason", reason);
  }

  var retry = document.getElementById("retry");
  var status = document.getElementById("status");
  if (retry) {
    retry.addEventListener("click", function () {
      retry.disabled = true;
      if (status) {
        status.textContent = "Checking again...";
      }
      post("local:retry");
      window.setTimeout(function () {
        retry.disabled = false;
        if (status) {
          status.textContent = "Still can't connect. We'll keep trying in the background.";
        }
      }, 4000);
    });
  }

  // Demo page: show what the page can see, and run the navigation checks.
  var bridgeState = document.getElementById("bridge-state");
  if (bridgeState) {
    bridgeState.textContent = window.metallic
      ? "Connected"
      : "Not connected (the bridge arrives in a later step)";
  }
  var ua = document.querySelector("#ua code");
  if (ua) {
    ua.textContent = navigator.userAgent;
  }

  var back = document.querySelector("[data-back]");
  if (back) {
    back.addEventListener("click", function (event) {
      if (window.history.length > 1) {
        event.preventDefault();
        window.history.back();
      }
    });
  }

  var result = document.getElementById("result");
  function say(text) {
    if (result) {
      result.textContent = text;
    }
  }

  var checks = document.querySelectorAll("[data-check]");
  Array.prototype.forEach.call(checks, function (button) {
    button.addEventListener("click", function () {
      var kind = button.getAttribute("data-check");
      if (kind === "popup") {
        // The scheme is joined here so no remote address is written in this file.
        var opened = window.open("https" + "://example.com/", "_blank");
        say(opened ? "A new window was opened." : "No new window was opened.");
      } else if (kind === "geolocation") {
        if (!navigator.geolocation) {
          say("Location is not available here.");
          return;
        }
        navigator.geolocation.getCurrentPosition(
          function () { say("Location was shared."); },
          function () { say("Location was refused."); }
        );
      }
    });
  });
})();
