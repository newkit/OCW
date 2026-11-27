// A fake WebUSB for the browser test of ANT+ (spec 14 Phase 17 A5,
// scripts/test-web-ant.py): `navigator.usb` with one ANT USB-m stick
// (0fcf:1009), injected before the page's scripts (DevTools
// `Page.addScriptToEvaluateOnNewDocument`). It behaves like Chromium's
// objects as far as the app uses them: requestDevice with filters (only
// in a user gesture: `navigator.userActivation.isActive`), getDevices for
// the sticks granted before (kept in sessionStorage, like Chromium's
// permission store), the `connect` / `disconnect` events, open,
// configuration (null until selectConfiguration), claimInterface,
// releaseInterface, close, transferIn / transferOut on the bulk endpoints
// 1 IN / 1 OUT with `{status, data}` / `{status, bytesWritten}`,
// clearHalt and reset; an unplugged stick rejects with NotFoundError.
//
// The stick speaks the simulated stick's bytes: the page's own
// `ocw_devices::ant::sim` (the desktop's `--ant-sim`) behind
// `window.ocw.antSim*` – it answers the setup and broadcasts the
// simulated heart-rate strap and power meter of `state.sensors` at their
// periods. It hears them only with the made-up test key (never the ANT+
// network key). The test unplugs it with `__ocwFakeUsb.unplug()` and plugs
// it in again with `__ocwFakeUsb.plug()`.
//
// With `#nousb` in the URL there is no WebUSB at all (Firefox / Safari).
(() => {
  if (location.hash.includes("nousb")) {
    Object.defineProperty(Navigator.prototype, "usb", { configurable: true, get: () => undefined });
    return;
  }
  const state = window.__ocwFakeUsb = {
    sensors: "hr:1234:141,power:5678:247:88",
    plugged: true, opens: 0, closes: 0, resets: 0, requests: [], outs: 0, ins: 0, log: [],
  };
  const now = () => Math.round(performance.now());
  const note = (t) => state.log.push(`${now()} ${t}`);
  const GRANTED = "ocw-fake-usb-granted";
  const granted = () => sessionStorage.getItem(GRANTED) === "1";
  const domError = (name, message) => new DOMException(message, name);
  let simStarted = false;
  const sim = () => {
    const ocw = window.ocw;
    if (!ocw || !ocw.antSimStart) throw domError("NetworkError", "the page's simulated stick is not loaded");
    if (!simStarted) { ocw.antSimStart(state.sensors); simStarted = true; }
    return ocw;
  };

  const endpoints = [
    { endpointNumber: 1, direction: "in", type: "bulk", packetSize: 64 },
    { endpointNumber: 1, direction: "out", type: "bulk", packetSize: 64 },
  ];
  const configuration = {
    configurationValue: 1, configurationName: null,
    interfaces: [{
      interfaceNumber: 0, claimed: false,
      alternate: { alternateSetting: 0, interfaceClass: 255, interfaceSubclass: 0, interfaceProtocol: 0, endpoints },
      alternates: [],
    }],
  };

  class Device {
    constructor() {
      this.vendorId = 0x0fcf; this.productId = 0x1009;
      this.productName = "ANT USB-m Stick"; this.manufacturerName = "Dynastream Innovations";
      this.serialNumber = "123";
      this.opened = false; this.configuration = null;
      this._gen = 0; this._pending = [];
    }
    _check() { if (!state.plugged) throw domError("NotFoundError", "The device was disconnected."); }
    async open() { this._check(); this.opened = true; state.opens++; note("open"); }
    async close() { this.opened = false; this.configuration = null; state.closes++; note("close"); }
    async selectConfiguration(v) {
      this._check();
      if (!this.opened) throw domError("InvalidStateError", "The device must be opened first.");
      if (v !== 1) throw domError("NotFoundError", "The configuration value provided is not supported by the device.");
      this.configuration = configuration;
    }
    async claimInterface(n) {
      this._check();
      if (!this.configuration) throw domError("InvalidStateError", "The device must have a configuration selected.");
      if (n !== 0) throw domError("NotFoundError", "The interface number provided is not supported by the device in its current configuration.");
      configuration.interfaces[0].claimed = true;
    }
    async releaseInterface() { configuration.interfaces[0].claimed = false; }
    async reset() { this._check(); state.resets++; note("reset"); }
    async clearHalt() { this._check(); }
    async transferOut(ep, data) {
      this._check();
      if (ep !== 1 || !configuration.interfaces[0].claimed) throw domError("InvalidStateError", "The specified endpoint is not part of a claimed and selected alternate interface.");
      const bytes = data instanceof ArrayBuffer ? new Uint8Array(data) : new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
      sim().antSimHost(new Uint8Array(bytes));
      state.outs++;
      return { status: "ok", bytesWritten: bytes.length };
    }
    transferIn(ep, length) {
      return new Promise((resolve, reject) => {
        const gen = this._gen;
        const poll = () => {
          if (!state.plugged || gen !== this._gen) {
            reject(domError("NotFoundError", "The device was disconnected."));
            return;
          }
          let bytes;
          try { bytes = sim().antSimRead(); } catch (e) { reject(e); return; }
          if (bytes.length) {
            if (bytes.length > length) {
              // The rest waits for the next transferIn, as a real stick's.
              this._rest = (this._rest || []).concat(Array.from(bytes.slice(length)));
              bytes = bytes.slice(0, length);
            }
            state.ins++;
            resolve({ status: "ok", data: new DataView(new Uint8Array(bytes).buffer) });
          } else if (this._rest && this._rest.length) {
            const part = this._rest.splice(0, length);
            state.ins++;
            resolve({ status: "ok", data: new DataView(new Uint8Array(part).buffer) });
          } else {
            setTimeout(poll, 10);
          }
        };
        try { this._check(); } catch (e) { reject(e); return; }
        if (ep !== 1) { reject(domError("InvalidStateError", "The specified endpoint is not part of a claimed and selected alternate interface.")); return; }
        poll();
      });
    }
  }
  const stick = new Device();

  class Usb extends EventTarget {
    async requestDevice(options) {
      state.requests.push({ t: now(), options, gesture: !!(navigator.userActivation && navigator.userActivation.isActive) });
      if (navigator.userActivation && !navigator.userActivation.isActive) {
        throw domError("SecurityError", "Must be handling a user gesture to show a permission request.");
      }
      const ok = (options.filters || []).some((f) => f.vendorId === stick.vendorId && (f.productId === undefined || f.productId === stick.productId));
      if (!ok || !state.plugged) throw domError("NotFoundError", "No device selected.");
      sessionStorage.setItem(GRANTED, "1");
      note("granted");
      return stick;
    }
    async getDevices() { return state.plugged && granted() ? [stick] : []; }
  }
  const usb = new Usb();
  Object.defineProperty(Navigator.prototype, "usb", { configurable: true, get: () => usb });

  const event = (type) => { const e = new Event(type); e.device = stick; return e; };
  state.unplug = () => {
    state.plugged = false; stick.opened = false; stick.configuration = null; stick._gen++;
    configuration.interfaces[0].claimed = false; stick._rest = [];
    note("unplugged");
    usb.dispatchEvent(event("disconnect"));
  };
  state.plug = () => {
    state.plugged = true;
    // A stick plugged in again starts fresh (the same sensors around).
    if (simStarted && window.ocw && window.ocw.antSimReplug) window.ocw.antSimReplug();
    note("plugged");
    usb.dispatchEvent(event("connect"));
  };
})();
