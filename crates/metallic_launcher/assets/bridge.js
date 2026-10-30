// The page side of the Launcher bridge. The Launcher injects this into the main frame before any
// page script runs, with the list of trusted origins filled in. It defines window.metallic.
//
// This check is a convenience for honest pages. The Launcher checks every message again on its
// side (origin, command, arguments, size), and that check is the one that counts.
(function () {
  "use strict";

  var ORIGINS = __METALLIC_ORIGINS__;
  var MAX_MESSAGE_CHARS = 16 * 1024;
  var MAX_IN_FLIGHT = 32;
  var TIMEOUT_MS = 60000;

  // Pages inside frames never get a bridge, and neither does an address outside the list.
  if (window.top !== window || ORIGINS.indexOf(window.location.origin) === -1) {
    return;
  }
  var ipc = window.ipc;
  if (!ipc || typeof ipc.postMessage !== "function") {
    return;
  }
  var post = ipc.postMessage.bind(ipc);

  // Ids start at a random point, so an answer meant for an earlier page in this window can
  // never match a call of this one.
  var nextId = 1 + Math.floor(Math.random() * 1000000000);
  var pending = Object.create(null);
  var pendingCount = 0;
  var listeners = Object.create(null);

  function fail(code) {
    return Promise.reject(Object.freeze({ code: code }));
  }

  function invoke(cmd, args) {
    if (typeof cmd !== "string") {
      return fail("badRequest");
    }
    if (pendingCount >= MAX_IN_FLIGHT) {
      return fail("busy");
    }
    var id = nextId++;
    var message = { id: id, cmd: cmd };
    if (args !== undefined) {
      message.args = args;
    }
    var text;
    try {
      text = JSON.stringify(message);
    } catch (error) {
      return fail("badRequest");
    }
    if (typeof text !== "string" || text.length > MAX_MESSAGE_CHARS) {
      return fail("badRequest");
    }
    return new Promise(function (resolve, reject) {
      var timer = window.setTimeout(function () {
        if (pending[id]) {
          delete pending[id];
          pendingCount--;
          reject(Object.freeze({ code: "failed" }));
        }
      }, TIMEOUT_MS);
      pending[id] = { resolve: resolve, reject: reject, timer: timer };
      pendingCount++;
      try {
        post(text);
      } catch (error) {
        window.clearTimeout(timer);
        delete pending[id];
        pendingCount--;
        reject(Object.freeze({ code: "failed" }));
      }
    });
  }

  function on(eventName, handler) {
    if (typeof eventName !== "string" || typeof handler !== "function") {
      return function () {};
    }
    var list = listeners[eventName] || (listeners[eventName] = []);
    list.push(handler);
    return function () {
      var at = list.indexOf(handler);
      if (at !== -1) {
        list.splice(at, 1);
      }
    };
  }

  // Called by the Launcher with the answer to a call. Unknown ids are ignored.
  function resolve(id, result) {
    var entry = pending[id];
    if (!entry) {
      return;
    }
    window.clearTimeout(entry.timer);
    delete pending[id];
    pendingCount--;
    if (result && Object.prototype.hasOwnProperty.call(result, "ok")) {
      entry.resolve(result.ok);
    } else {
      var code = result && typeof result.error === "string" ? result.error : "failed";
      entry.reject(Object.freeze({ code: code }));
    }
  }

  // Called by the Launcher with an event: {event: "<name>", ...payload}.
  function emit(event) {
    if (!event || typeof event.event !== "string") {
      return;
    }
    var payload = {};
    Object.keys(event).forEach(function (key) {
      if (key !== "event") {
        payload[key] = event[key];
      }
    });
    var list = listeners[event.event];
    if (!list) {
      return;
    }
    list.slice().forEach(function (handler) {
      try {
        handler(payload);
      } catch (error) {
        // One bad handler must not stop the others.
      }
    });
  }

  var locked = { writable: false, configurable: false, enumerable: false };
  Object.defineProperty(window, "__metallicBridge", Object.assign({
    value: Object.freeze({ resolve: resolve, emit: emit })
  }, locked));
  Object.defineProperty(window, "metallic", {
    value: Object.freeze({ bridgeVersion: 1, invoke: invoke, on: on }),
    writable: false,
    configurable: false,
    enumerable: true
  });
})();
