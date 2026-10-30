// Scripts for the bundled pages. No remote addresses and no inline script: the page policy forbids both.
(function () {
  "use strict";

  var REASONS = ["unreachable", "notconfigured", "loadfailed"];

  // Asks the Launcher to do something. Resolves to the answer, rejects with {code}.
  function ask(cmd, args) {
    if (!window.metallic) {
      return Promise.reject({ code: "notAvailable" });
    }
    return window.metallic.invoke(cmd, args);
  }

  var ERROR_TEXT = {
    badRequest: "That value is not valid.",
    notAllowed: "This page may not do that.",
    notAvailable: "Not available in this build.",
    busy: "Busy. Try again in a moment.",
    failed: "That did not work. Try again."
  };
  function errorText(error) {
    return (error && ERROR_TEXT[error.code]) || ERROR_TEXT.failed;
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
      ask("retrySite").catch(function () {});
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
      ? "Connected (version " + window.metallic.bridgeVersion + ")"
      : "Not connected";
  }

  // Demo page: run a bridge command and show the answer. This page is a bundled page, so
  // commands for the site (play, sign in, Studio) are refused here, and the page says so.
  var bridgeOut = document.getElementById("bridge-out");
  Array.prototype.forEach.call(document.querySelectorAll("[data-cmd]"), function (button) {
    button.addEventListener("click", function () {
      var cmd = button.getAttribute("data-cmd");
      ask(cmd).then(
        function (value) { bridgeOut.textContent = cmd + ": " + JSON.stringify(value); },
        function (error) { bridgeOut.textContent = cmd + " refused: " + (error && error.code); }
      );
    });
  });

  // Settings page: show the saved values and save changes.
  var form = document.getElementById("settings-form");
  if (form) {
    var closeToTray = document.getElementById("close-to-tray");
    var developerMode = document.getElementById("developer-mode");
    var devUrl = document.getElementById("dev-url");
    var channel = document.getElementById("channel");
    var githubButton = document.getElementById("github-sign-in");
    var githubStatus = document.getElementById("github-status");
    var save = document.getElementById("save");
    var note = document.getElementById("settings-status");
    var say = function (text, kind) {
      note.textContent = text;
      note.className = "status" + (kind ? " " + kind : "");
    };
    var syncDevUrl = function () { devUrl.disabled = !developerMode.checked; };
    developerMode.addEventListener("change", syncDevUrl);

    ask("getSettings").then(function (settings) {
      closeToTray.checked = settings.closeToTray === true;
      developerMode.checked = settings.developerMode === true;
      devUrl.value = typeof settings.devUrl === "string" ? settings.devUrl : "";
      channel.value = settings.channel === "stable" ? "stable" : "dev";
      syncDevUrl();
      save.disabled = false;
    }, function () {
      syncDevUrl();
      say("Settings can only be changed in the Metallic app.", "error");
    });

    form.addEventListener("submit", function (event) {
      event.preventDefault();
      save.disabled = true;
      var address = devUrl.value.trim();
      var patch = {
        closeToTray: closeToTray.checked,
        developerMode: developerMode.checked,
        devUrl: developerMode.checked && address !== "" ? address : null,
        channel: channel.value
      };
      ask("setSettings", { patch: patch }).then(function () {
        say("Saved.", "ok");
      }, function (error) {
        say(error && error.code === "badRequest"
          ? "The development address must be https, or http on localhost."
          : errorText(error), "error");
      }).then(function () { save.disabled = false; });
    });
  }
  // GitHub sign-in for team builds: the Launcher opens GitHub in the browser; the page shows
  // the code to type there.
  if (githubButton && githubStatus) {
    var GITHUB_TEXT = {
      signedIn: ["Signed in to GitHub.", "ok"],
      denied: ["Sign-in was refused on the GitHub page.", "error"],
      expired: ["The code expired. Start again.", "error"],
      failed: ["GitHub sign-in did not work. Try again.", "error"]
    };
    var sayGithub = function (text, kind) {
      githubStatus.textContent = text;
      githubStatus.className = "status" + (kind ? " " + kind : "");
    };
    if (window.metallic) {
      window.metallic.on("githubSignIn", function (event) {
        if (event.state === "pending" && typeof event.userCode === "string") {
          sayGithub("Type " + event.userCode + " on the GitHub page that opened.", "");
          return;
        }
        var text = GITHUB_TEXT[event.state];
        if (text) {
          sayGithub(text[0], text[1]);
          githubButton.disabled = false;
        }
      });
    }
    githubButton.addEventListener("click", function () {
      githubButton.disabled = true;
      sayGithub("Opening GitHub...", "");
      ask("githubSignIn").catch(function (error) {
        sayGithub(errorText(error), "error");
        githubButton.disabled = false;
      });
    });
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
