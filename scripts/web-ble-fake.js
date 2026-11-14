// A fake Web Bluetooth for the browser test (spec §14 Phase 10 B4,
// scripts/test-web-ble.py): `navigator.bluetooth` with an FTMS smart
// trainer ("OCW Fake FTMS") and a heart-rate strap ("OCW Fake HR"),
// injected before the page's scripts (DevTools `Page.addScriptToEvaluate
// OnNewDocument`). It behaves like Chromium's objects as far as the app
// uses them: requestDevice with filters, gatt.connect / disconnect,
// getPrimaryServices, getCharacteristics, readValue,
// writeValueWithResponse, startNotifications with
// `characteristicvaluechanged`, `gattserverdisconnected`, getDevices for
// the devices permitted before (kept in sessionStorage, like Chromium's
// permission persistence) and getAvailability.
//
// The trainer reports Indoor Bike Data at 4 Hz (power `state.power` W,
// `state.cadence` rpm; in ERG mode it holds the target like an ideal ERG
// trainer), answers every control point write with a success indication,
// and records the writes in `window.__ocwFakeBle.writes`. The test drops
// it with `__ocwFakeBle.drop("trainer", ms)` (out of range for `ms`).
//
// With `#nobt` in the URL there is no Web Bluetooth at all (Firefox /
// Safari).
(() => {
  if (location.hash.includes("nobt")) {
    Object.defineProperty(Navigator.prototype, "bluetooth", { configurable: true, get: () => undefined });
    return;
  }
  const S = (n) => `0000${n.toString(16).padStart(4, "0")}-0000-1000-8000-00805f9b34fb`;
  const U = {
    ftms: S(0x1826), feature: S(0x2acc), bike: S(0x2ad2), control: S(0x2ad9), status: S(0x2ada),
    hrs: S(0x180d), hrm: S(0x2a37), dis: S(0x180a), mfr: S(0x2a29),
  };
  const state = window.__ocwFakeBle = {
    power: 321, cadence: 88, hr: 142, erg: null,
    writes: [], connects: { trainer: 0, hr: 0 }, requests: [], log: [],
  };
  const now = () => Math.round(performance.now());
  const note = (t) => state.log.push(`${now()} ${t}`);
  const bytesOf = (v) => v instanceof ArrayBuffer
    ? Array.from(new Uint8Array(v))
    : Array.from(new Uint8Array(v.buffer, v.byteOffset, v.byteLength));
  const view = (bytes) => new DataView(new Uint8Array(bytes).buffer);
  const props = (p) => Object.assign({
    broadcast: false, read: false, writeWithoutResponse: false, write: false,
    notify: false, indicate: false, authenticatedSignedWrites: false,
    reliableWrite: false, writableAuxiliaries: false,
  }, p);

  class Characteristic extends EventTarget {
    constructor(device, service, uuid, properties, read) {
      super();
      this.service = service; this.uuid = uuid; this.properties = props(properties);
      this.value = null; this._device = device; this._read = read; this._notifying = false;
    }
    async readValue() {
      this._device._check();
      this.value = view(this._read ? this._read() : []);
      return this.value;
    }
    async writeValueWithResponse(v) {
      this._device._check();
      const bytes = bytesOf(v);
      state.writes.push({ t: now(), device: this._device._key, uuid: this.uuid, bytes });
      setTimeout(() => this._device._written(this, bytes), 20);
    }
    async writeValueWithoutResponse(v) { return this.writeValueWithResponse(v); }
    async writeValue(v) { return this.writeValueWithResponse(v); }
    async startNotifications() {
      this._device._check();
      this._notifying = true;
      note(`${this._device._key} notify ${this.uuid.slice(4, 8)}`);
      return this;
    }
    async stopNotifications() { this._notifying = false; return this; }
    _emit(bytes) {
      if (!this._notifying || !this._device.gatt.connected) return;
      this.value = view(bytes);
      this.dispatchEvent(new Event("characteristicvaluechanged"));
    }
  }

  class Service {
    constructor(device, uuid) { this.device = device; this.uuid = uuid; this.isPrimary = true; this._chars = []; }
    async getCharacteristics(uuid) {
      this.device._check();
      const list = uuid ? this._chars.filter((c) => c.uuid === uuid) : this._chars.slice();
      if (!list.length) throw new DOMException("No Characteristics matching UUID found in Service.", "NotFoundError");
      return list;
    }
    async getCharacteristic(uuid) { return (await this.getCharacteristics(uuid))[0]; }
  }

  class Server {
    constructor(device) { this.device = device; this.connected = false; }
    async connect() {
      await new Promise((r) => setTimeout(r, 50));
      if (now() < this.device._awayUntil) {
        note(`${this.device._key} connect refused (out of range)`);
        throw new DOMException("Bluetooth Device is no longer in range.", "NetworkError");
      }
      this.connected = true;
      state.connects[this.device._key]++;
      note(`${this.device._key} connected`);
      return this;
    }
    disconnect() {
      if (!this.connected) return;
      this.connected = false;
      note(`${this.device._key} disconnect() by the page`);
      this.device.dispatchEvent(new Event("gattserverdisconnected"));
    }
    async getPrimaryServices(uuid) {
      this.device._check();
      const list = uuid ? this.device._services.filter((s) => s.uuid === uuid) : this.device._services.slice();
      if (!list.length) throw new DOMException("No Services matching UUID found in Device.", "NotFoundError");
      return list;
    }
    async getPrimaryService(uuid) { return (await this.getPrimaryServices(uuid))[0]; }
  }

  class Device extends EventTarget {
    constructor(key, id, name) {
      super();
      this._key = key; this.id = id; this.name = name; this._services = []; this._awayUntil = 0;
      this.gatt = new Server(this);
    }
    _check() {
      if (!this.gatt.connected) throw new DOMException("GATT Server is disconnected.", "NetworkError");
    }
    _service(uuid) { const s = new Service(this, uuid); this._services.push(s); return s; }
    _char(service, uuid, p, read) { const c = new Characteristic(this, service, uuid, p, read); service._chars.push(c); return c; }
    _written() {}
    async watchAdvertisements() {}
    async forget() {}
  }

  const devices = {};
  function trainer() {
    if (devices.trainer) return devices.trainer;
    const d = devices.trainer = new Device("trainer", "fake-ftms-0001", "OCW Fake FTMS");
    const info = d._service(U.dis);
    d._char(info, U.mfr, { read: true }, () => [...new TextEncoder().encode("OCW")]);
    const ftms = d._service(U.ftms);
    // Cadence and power measured; power (ERG) and simulation targets.
    d._char(ftms, U.feature, { read: true }, () => [0x02, 0x40, 0, 0, 0x08, 0x20, 0, 0]);
    const bike = d._char(ftms, U.bike, { notify: true });
    const control = d._char(ftms, U.control, { write: true, indicate: true });
    d._char(ftms, U.status, { notify: true });
    d._written = (c, bytes) => {
      if (c !== control) return;
      const op = bytes[0];
      if (op === 0x05) state.erg = bytes[1] | (bytes[2] << 8);
      if (op === 0x11) state.erg = null;
      control._emit([0x80, op, 0x01]);
    };
    setInterval(() => {
      const w = state.erg ?? state.power;
      const speed = 3000; // 30 km/h in 0.01 km/h
      const c = state.cadence * 2;
      bike._emit([0x44, 0x00, speed & 255, speed >> 8, c & 255, c >> 8, w & 255, (w >> 8) & 255]);
    }, 250);
    return d;
  }
  function strap() {
    if (devices.hr) return devices.hr;
    const d = devices.hr = new Device("hr", "fake-hr-0001", "OCW Fake HR");
    const hrm = d._char(d._service(U.hrs), U.hrm, { notify: true });
    setInterval(() => hrm._emit([0x00, state.hr]), 1000);
    return d;
  }
  const make = { trainer, hr: strap };

  const PERMITTED = "ocwFakeBlePermitted";
  const permitted = () => { try { return JSON.parse(sessionStorage.getItem(PERMITTED) || "[]"); } catch (e) { return []; } };

  // The test's controls.
  state.drop = (key, ms) => {
    const d = devices[key];
    if (!d) return false;
    d._awayUntil = now() + (ms || 0);
    d.gatt.connected = false;
    note(`${key} dropped for ${ms} ms`);
    d.dispatchEvent(new Event("gattserverdisconnected"));
    return true;
  };
  state.writesOf = (key) => state.writes.filter((w) => w.device === key);

  const bluetooth = new EventTarget();
  bluetooth.getAvailability = async () => true;
  bluetooth.getDevices = async () => permitted().map((k) => make[k]());
  bluetooth.requestDevice = (options) => {
    state.requests.push(JSON.parse(JSON.stringify(options || {})));
    const wanted = (options && options.filters || []).flatMap((f) => f.services || []);
    const key = wanted.includes(U.hrs) ? "hr" : "trainer";
    note(`requestDevice -> ${key} (user activation: ${navigator.userActivation ? navigator.userActivation.isActive : "?"})`);
    return new Promise((resolve, reject) => setTimeout(() => {
      if (state.cancel) {
        reject(new DOMException("User cancelled the requestDevice() chooser.", "NotFoundError"));
        return;
      }
      const list = permitted();
      if (!list.includes(key)) { list.push(key); sessionStorage.setItem(PERMITTED, JSON.stringify(list)); }
      resolve(make[key]());
    }, 200));
  };
  Object.defineProperty(Navigator.prototype, "bluetooth", { configurable: true, get: () => bluetooth });
})();
