(() => {
  const NativeReadableStream = ReadableStream;
  // BYOB and byte controllers are deliberately outside the supported subset.
  const getReader = NativeReadableStream.prototype.getReader;
  NativeReadableStream.prototype.getReader = function (options = {}) {
    if (options.mode !== undefined) throw new TypeError("BYOB is unsupported");
    return getReader.call(this);
  };
  globalThis.ReadableStream = new Proxy(NativeReadableStream, {
    construct(target, [source = {}, strategy], newTarget) {
      if (source.type !== undefined)
        throw new TypeError("Byte streams/BYOB are unsupported");
      return Reflect.construct(target, [source, strategy], newTarget);
    },
  });
  for (const name of [
    "ReadableStreamBYOBReader",
    "ReadableStreamBYOBRequest",
    "ReadableByteStreamController",
  ])
    delete globalThis[name];
  globalThis.TextEncoderStream = class TextEncoderStream {
    constructor() {
      const encoder = new TextEncoder();
      let pending = "";
      const stream = new TransformStream({
        transform(chunk, controller) {
          let text = pending + String(chunk);
          pending = "";
          if (text.length && /[\uD800-\uDBFF]/.test(text[text.length - 1])) {
            pending = text.slice(-1);
            text = text.slice(0, -1);
          }
          if (text) controller.enqueue(encoder.encode(text));
        },
        flush(controller) {
          if (pending) controller.enqueue(encoder.encode(pending));
        },
      });
      this.readable = stream.readable;
      this.writable = stream.writable;
    }
    get encoding() {
      return "utf-8";
    }
  };
  globalThis.TextDecoderStream = class TextDecoderStream {
    constructor(label = "utf-8", options) {
      const decoder = new TextDecoder(label, options);
      const stream = new TransformStream({
        transform(chunk, controller) {
          const text = decoder.decode(chunk, { stream: true });
          if (text) controller.enqueue(text);
        },
        flush(controller) {
          const text = decoder.decode();
          if (text) controller.enqueue(text);
        },
      });
      this.readable = stream.readable;
      this.writable = stream.writable;
      this.encoding = decoder.encoding;
      this.fatal = decoder.fatal;
      this.ignoreBOM = decoder.ignoreBOM;
    }
  };
})();
