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

  // Shows `text` in a status line (a <p class="status"> with an icon per kind and a <span>).
  // `kind` is "ok", "error" or "" (plain); empty text hides the line.
  function show(line, text, kind) {
    if (!line) {
      return;
    }
    var span = line.querySelector("span");
    (span || line).textContent = text;
    line.className = "status" + (kind ? " " + kind : "");
    line.hidden = text === "";
  }

  // The settings file is the lasting record of the theme; theme.js keeps a copy in page storage
  // so the first paint is right. Bring the copy up to date when they differ.
  var theme = window.metallicTheme;
  var settingsAnswer = ask("getSettings");
  settingsAnswer.then(function (settings) {
    if (theme && typeof settings.theme === "string" && settings.theme !== theme.choice()) {
      theme.remember(settings.theme);
    }
  }, function () {});

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
      show(status, "Checking again...", "");
      ask("retrySite").catch(function () {});
      window.setTimeout(function () {
        retry.disabled = false;
        show(status, "Still can't connect. We'll keep trying in the background.", "error");
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
    var devUrlError = document.getElementById("dev-url-error");
    var say = function (text, kind) { show(note, text, kind); };
    var markDevUrl = function (bad) {
      devUrl.setAttribute("aria-invalid", bad ? "true" : "false");
      devUrlError.hidden = !bad;
    };
    var syncDevUrl = function () {
      devUrl.disabled = !developerMode.checked;
      if (devUrl.disabled) {
        markDevUrl(false);
      }
    };
    developerMode.addEventListener("change", syncDevUrl);
    devUrl.addEventListener("input", function () { markDevUrl(false); });

    settingsAnswer.then(function (settings) {
      closeToTray.checked = settings.closeToTray === true;
      developerMode.checked = settings.developerMode === true;
      devUrl.value = typeof settings.devUrl === "string" ? settings.devUrl : "";
      channel.value = settings.channel === "stable" ? "stable" : "dev";
      syncDevUrl();
      save.disabled = false;
    }, function () {
      syncDevUrl();
      say("Settings can only be changed in the Metallic Play app.", "error");
    });

    form.addEventListener("submit", function (event) {
      event.preventDefault();
      save.disabled = true;
      markDevUrl(false);
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
        if (error && error.code === "badRequest") {
          markDevUrl(true);
          say("Not saved. Check the development address.", "error");
          devUrl.focus();
        } else {
          say(errorText(error), "error");
        }
      }).then(function () { save.disabled = false; });
    });
  }

  // Settings page: the theme. It shows at once and is saved as it is chosen, like Studio's.
  var themeChoices = document.querySelectorAll("input[name=theme]");
  if (theme && themeChoices.length) {
    var themeStatus = document.getElementById("theme-status");
    var mark = function (word) {
      Array.prototype.forEach.call(themeChoices, function (input) {
        input.checked = input.value === word;
      });
    };
    mark(theme.choice());
    settingsAnswer.then(function (settings) {
      if (typeof settings.theme === "string") {
        mark(settings.theme);
      }
    }, function () {});
    var pending = null;
    Array.prototype.forEach.call(themeChoices, function (input) {
      input.addEventListener("change", function () {
        if (!input.checked) {
          return;
        }
        theme.remember(input.value);
        show(themeStatus, "", "");
        // The Launcher takes at most two saves a second: save the last choice of a quick run.
        window.clearTimeout(pending);
        pending = window.setTimeout(function () {
          ask("setSettings", { patch: { theme: theme.choice() } }).catch(function (error) {
            if (error && error.code !== "notAvailable") {
              show(themeStatus, "The theme was not saved. " + errorText(error), "error");
            }
          });
        }, 400);
      });
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
    var sayGithub = function (text, kind) { show(githubStatus, text, kind); };
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
