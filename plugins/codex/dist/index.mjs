// node_modules/hono/dist/compose.js
var compose = (middleware, onError, onNotFound) => {
  return (context, next) => {
    let index = -1;
    return dispatch(0);
    async function dispatch(i) {
      if (i <= index) {
        throw new Error("next() called multiple times");
      }
      index = i;
      let res;
      let isError = false;
      let handler;
      if (middleware[i]) {
        handler = middleware[i][0][0];
        context.req.routeIndex = i;
      } else {
        handler = i === middleware.length && next || void 0;
      }
      if (handler) {
        try {
          res = await handler(context, () => dispatch(i + 1));
        } catch (err) {
          if (err instanceof Error && onError) {
            context.error = err;
            res = await onError(err, context);
            isError = true;
          } else {
            throw err;
          }
        }
      } else {
        if (context.finalized === false && onNotFound) {
          res = await onNotFound(context);
        }
      }
      if (res && (context.finalized === false || isError)) {
        context.res = res;
      }
      return context;
    }
  };
};

// node_modules/hono/dist/request/constants.js
var GET_MATCH_RESULT = /* @__PURE__ */ Symbol();

// node_modules/hono/dist/utils/buffer.js
var bufferToFormData = (arrayBuffer, contentType) => {
  const response = new Response(arrayBuffer, {
    headers: {
      // Normalize the media type (case-insensitive) while keeping parameters like the boundary
      "Content-Type": contentType.replace(/^[^;]+/, (mediaType) => mediaType.toLowerCase())
    }
  });
  return response.formData();
};

// node_modules/hono/dist/utils/body.js
var MAX_NESTING_DEPTH = 32;
var MAX_NESTED_OBJECTS = 1e4;
var isRawRequest = (request) => "headers" in request;
var parseBody = async (request, options = /* @__PURE__ */ Object.create(null)) => {
  const { all = false, dot = false } = options;
  const headers = isRawRequest(request) ? request.headers : request.raw.headers;
  const contentType = headers.get("Content-Type");
  const mediaType = contentType?.split(";")[0].trim().toLowerCase();
  if (mediaType === "multipart/form-data" || mediaType === "application/x-www-form-urlencoded") {
    return parseFormData(request, { all, dot });
  }
  return {};
};
async function parseFormData(request, options) {
  if (!isRawRequest(request) && request.bodyCache.formData) {
    return convertFormDataToBodyData(
      await request.bodyCache.formData,
      options
    );
  }
  const headers = isRawRequest(request) ? request.headers : request.raw.headers;
  const arrayBuffer = await request.arrayBuffer();
  const formDataPromise = bufferToFormData(arrayBuffer, headers.get("Content-Type") || "");
  if (!isRawRequest(request)) {
    request.bodyCache.formData = formDataPromise;
  }
  const formData = await formDataPromise;
  if (formData) {
    return convertFormDataToBodyData(formData, options);
  }
  return {};
}
function convertFormDataToBodyData(formData, options) {
  const form = /* @__PURE__ */ Object.create(null);
  const nestingState = { count: 0 };
  formData.forEach((value, key) => {
    const shouldParseAllValues = options.all || key.endsWith("[]");
    if (!shouldParseAllValues) {
      form[key] = value;
    } else {
      handleParsingAllValues(form, key, value);
    }
  });
  if (options.dot) {
    Object.entries(form).forEach(([key, value]) => {
      const shouldParseDotValues = key.includes(".");
      if (shouldParseDotValues) {
        handleParsingNestedValues(form, key, value, nestingState);
        delete form[key];
      }
    });
  }
  return form;
}
var handleParsingAllValues = (form, key, value) => {
  if (form[key] !== void 0) {
    if (Array.isArray(form[key])) {
      ;
      form[key].push(value);
    } else {
      form[key] = [form[key], value];
    }
  } else {
    if (!key.endsWith("[]")) {
      form[key] = value;
    } else {
      form[key] = [value];
    }
  }
};
var handleParsingNestedValues = (form, key, value, state) => {
  if (/(?:^|\.)__proto__\./.test(key)) {
    return;
  }
  let nestedForm = form;
  const keys = key.split(".", MAX_NESTING_DEPTH + 2);
  if (keys.length > MAX_NESTING_DEPTH + 1) {
    throwNestingLimitExceeded();
  }
  keys.forEach((key2, index) => {
    if (index === keys.length - 1) {
      nestedForm[key2] = value;
    } else {
      if (!nestedForm[key2] || typeof nestedForm[key2] !== "object" || Array.isArray(nestedForm[key2]) || nestedForm[key2] instanceof File) {
        if (state.count++ >= MAX_NESTED_OBJECTS) {
          throwNestingLimitExceeded();
        }
        nestedForm[key2] = /* @__PURE__ */ Object.create(null);
      }
      nestedForm = nestedForm[key2];
    }
  });
};
var throwNestingLimitExceeded = () => {
  throw new Error("Nesting limit exceeded");
};

// node_modules/hono/dist/utils/url.js
var splitPath = (path) => {
  const paths = path.split("/");
  if (paths[0] === "") {
    paths.shift();
  }
  return paths;
};
var splitRoutingPath = (routePath) => {
  const { groups, path } = extractGroupsFromPath(routePath);
  const paths = splitPath(path);
  return replaceGroupMarks(paths, groups);
};
var extractGroupsFromPath = (path) => {
  const groups = [];
  path = path.replace(/\{[^}]+\}/g, (match, index) => {
    const mark = `@${index}`;
    groups.push([mark, match]);
    return mark;
  });
  return { groups, path };
};
var replaceGroupMarks = (paths, groups) => {
  for (let i = groups.length - 1; i >= 0; i--) {
    const [mark] = groups[i];
    for (let j = paths.length - 1; j >= 0; j--) {
      if (paths[j].includes(mark)) {
        paths[j] = paths[j].replace(mark, groups[i][1]);
        break;
      }
    }
  }
  return paths;
};
var patternCache = {};
var getPattern = (label, next) => {
  if (label === "*") {
    return "*";
  }
  const match = label.match(/^\:([^\{\}]+)(?:\{(.+)\})?$/);
  if (match) {
    const cacheKey = `${label}#${next}`;
    if (!patternCache[cacheKey]) {
      if (match[2]) {
        patternCache[cacheKey] = next && next[0] !== ":" && next[0] !== "*" ? [cacheKey, match[1], new RegExp(`^${match[2]}(?=/${next})`)] : [label, match[1], new RegExp(`^${match[2]}$`)];
      } else {
        patternCache[cacheKey] = [label, match[1], true];
      }
    }
    return patternCache[cacheKey];
  }
  return null;
};
var tryDecode = (str, decoder) => {
  try {
    return decoder(str);
  } catch {
    return str.replace(/(?:%[0-9A-Fa-f]{2})+/g, (match) => {
      try {
        return decoder(match);
      } catch {
        return match;
      }
    });
  }
};
var tryDecodeURI = (str) => tryDecode(str, decodeURI);
var getPath = (request) => {
  const url = request.url;
  const start = url.indexOf("/", url.indexOf(":") + 4);
  let i = start;
  for (; i < url.length; i++) {
    const charCode = url.charCodeAt(i);
    if (charCode === 37) {
      const queryIndex = url.indexOf("?", i);
      const hashIndex = url.indexOf("#", i);
      const end = queryIndex === -1 ? hashIndex === -1 ? void 0 : hashIndex : hashIndex === -1 ? queryIndex : Math.min(queryIndex, hashIndex);
      const path = url.slice(start, end);
      return tryDecodeURI(path.includes("%25") ? path.replace(/%25/g, "%2525") : path);
    } else if (charCode === 63 || charCode === 35) {
      break;
    }
  }
  return url.slice(start, i);
};
var getPathNoStrict = (request) => {
  const result = getPath(request);
  return result.length > 1 && result.at(-1) === "/" ? result.slice(0, -1) : result;
};
var mergePath = (base, sub, ...rest) => {
  if (rest.length) {
    sub = mergePath(sub, ...rest);
  }
  return `${base?.[0] === "/" ? "" : "/"}${base}${sub === "/" ? "" : `${base?.at(-1) === "/" ? "" : "/"}${sub?.[0] === "/" ? sub.slice(1) : sub}`}`;
};
var checkOptionalParameter = (path) => {
  if (path.charCodeAt(path.length - 1) !== 63 || !path.includes(":")) {
    return null;
  }
  const segments = path.split("/");
  const results = [];
  let basePath = "";
  segments.forEach((segment) => {
    if (segment !== "" && !/\:/.test(segment)) {
      basePath += "/" + segment;
    } else if (/\:/.test(segment)) {
      if (segment.charCodeAt(segment.length - 1) === 63) {
        if (results.length === 0 && basePath === "") {
          results.push("/");
        } else {
          results.push(basePath);
        }
        const optionalSegment = segment.slice(0, -1);
        basePath += "/" + optionalSegment;
        results.push(basePath);
      } else {
        basePath += "/" + segment;
      }
    }
  });
  return results.filter((v, i, a) => a.indexOf(v) === i);
};
var tryDecodeURIComponent = (str) => str.indexOf("%") !== -1 ? tryDecode(str, decodeURIComponent_) : str;
var _decodeURI = (value) => {
  if (value.indexOf("+") !== -1) {
    value = value.replace(/\+/g, " ");
  }
  return tryDecodeURIComponent(value);
};
var _getQueryParam = (url, key, multiple) => {
  const hashIndex = url.indexOf("#", 8);
  if (hashIndex !== -1) {
    url = url.slice(0, hashIndex);
  }
  let encoded;
  if (!multiple && key && key.indexOf("%") === -1 && key.indexOf("+") === -1) {
    let keyIndex2 = url.indexOf("?", 8);
    if (keyIndex2 === -1) {
      return void 0;
    }
    if (!url.startsWith(key, keyIndex2 + 1)) {
      keyIndex2 = url.indexOf(`&${key}`, keyIndex2 + 1);
    }
    while (keyIndex2 !== -1) {
      const trailingKeyCode = url.charCodeAt(keyIndex2 + key.length + 1);
      if (trailingKeyCode === 61) {
        const valueIndex = keyIndex2 + key.length + 2;
        const endIndex = url.indexOf("&", valueIndex);
        return _decodeURI(url.slice(valueIndex, endIndex === -1 ? void 0 : endIndex));
      } else if (trailingKeyCode == 38 || isNaN(trailingKeyCode)) {
        return "";
      }
      keyIndex2 = url.indexOf(`&${key}`, keyIndex2 + 1);
    }
    encoded = /[%+]/.test(url);
    if (!encoded) {
      return void 0;
    }
  }
  const results = /* @__PURE__ */ Object.create(null);
  encoded ??= /[%+]/.test(url);
  let keyIndex = url.indexOf("?", 8);
  while (keyIndex !== -1) {
    const nextKeyIndex = url.indexOf("&", keyIndex + 1);
    let valueIndex = url.indexOf("=", keyIndex);
    if (valueIndex > nextKeyIndex && nextKeyIndex !== -1) {
      valueIndex = -1;
    }
    let name = url.slice(
      keyIndex + 1,
      valueIndex === -1 ? nextKeyIndex === -1 ? void 0 : nextKeyIndex : valueIndex
    );
    if (encoded) {
      name = _decodeURI(name);
    }
    keyIndex = nextKeyIndex;
    if (name === "") {
      continue;
    }
    let value;
    if (valueIndex === -1) {
      value = "";
    } else {
      value = url.slice(valueIndex + 1, nextKeyIndex === -1 ? void 0 : nextKeyIndex);
      if (encoded) {
        value = _decodeURI(value);
      }
    }
    if (multiple) {
      if (!(results[name] && Array.isArray(results[name]))) {
        results[name] = [];
      }
      ;
      results[name].push(value);
    } else {
      results[name] ??= value;
    }
  }
  return key ? results[key] : results;
};
var getQueryParam = _getQueryParam;
var getQueryParams = (url, key) => {
  return _getQueryParam(url, key, true);
};
var decodeURIComponent_ = decodeURIComponent;

// node_modules/hono/dist/request.js
var HonoRequest = class {
  /**
   * `.raw` can get the raw Request object.
   *
   * @see {@link https://hono.dev/docs/api/request#raw}
   *
   * @example
   * ```ts
   * // For Cloudflare Workers
   * app.post('/', async (c) => {
   *   const metadata = c.req.raw.cf?.hostMetadata?
   *   ...
   * })
   * ```
   */
  raw;
  #validatedData;
  // Short name of validatedData
  #matchResult;
  routeIndex = 0;
  /**
   * `.path` can get the pathname of the request.
   *
   * @see {@link https://hono.dev/docs/api/request#path}
   *
   * @example
   * ```ts
   * app.get('/about/me', (c) => {
   *   const pathname = c.req.path // `/about/me`
   * })
   * ```
   */
  path;
  bodyCache = {};
  constructor(request, path = "/", matchResult = [[]]) {
    this.raw = request;
    this.path = path;
    this.#matchResult = matchResult;
  }
  param(key) {
    return key ? this.#getDecodedParam(key) : this.#getAllDecodedParams();
  }
  #getDecodedParam(key) {
    const paramKey = this.#matchResult[0][this.routeIndex]?.[1][key];
    const param = this.#getParamValue(paramKey);
    return param && tryDecodeURIComponent(param);
  }
  #getAllDecodedParams() {
    const decoded = {};
    const keys = Object.keys(this.#matchResult[0][this.routeIndex]?.[1] ?? {});
    for (const key of keys) {
      const value = this.#getParamValue(this.#matchResult[0][this.routeIndex][1][key]);
      if (value !== void 0) {
        decoded[key] = tryDecodeURIComponent(value);
      }
    }
    return decoded;
  }
  #getParamValue(paramKey) {
    return this.#matchResult[1] ? this.#matchResult[1][paramKey] : paramKey;
  }
  query(key) {
    return getQueryParam(this.url, key);
  }
  queries(key) {
    return getQueryParams(this.url, key);
  }
  header(name) {
    if (name) {
      return this.raw.headers.get(name) ?? void 0;
    }
    const headerData = /* @__PURE__ */ Object.create(null);
    this.raw.headers.forEach((value, key) => {
      headerData[key] = value;
    });
    return headerData;
  }
  async parseBody(options) {
    return parseBody(this, options);
  }
  #cachedBody = (key) => {
    const { bodyCache, raw: raw2 } = this;
    const cachedBody = bodyCache[key];
    if (cachedBody) {
      return cachedBody;
    }
    for (const anyCachedKey in bodyCache) {
      return bodyCache[anyCachedKey].then((body) => {
        if (anyCachedKey === "json") {
          body = JSON.stringify(body);
        }
        const contentType = anyCachedKey === "formData" ? void 0 : raw2.headers.get("content-type");
        return new Response(body, {
          headers: contentType ? { "Content-Type": contentType } : void 0
        })[key]();
      });
    }
    return bodyCache[key] = raw2[key]();
  };
  /**
   * `.json()` can parse Request body of type `application/json`
   *
   * @see {@link https://hono.dev/docs/api/request#json}
   *
   * @example
   * ```ts
   * app.post('/entry', async (c) => {
   *   const body = await c.req.json()
   * })
   * ```
   */
  json() {
    return this.#cachedBody("text").then((text) => JSON.parse(text));
  }
  /**
   * `.text()` can parse Request body of type `text/plain`
   *
   * @see {@link https://hono.dev/docs/api/request#text}
   *
   * @example
   * ```ts
   * app.post('/entry', async (c) => {
   *   const body = await c.req.text()
   * })
   * ```
   */
  text() {
    return this.#cachedBody("text");
  }
  /**
   * `.arrayBuffer()` parse Request body as an `ArrayBuffer`
   *
   * @see {@link https://hono.dev/docs/api/request#arraybuffer}
   *
   * @example
   * ```ts
   * app.post('/entry', async (c) => {
   *   const body = await c.req.arrayBuffer()
   * })
   * ```
   */
  arrayBuffer() {
    return this.#cachedBody("arrayBuffer");
  }
  /**
   * `.bytes()` parses the request body as a `Uint8Array`.
   *
   * @see {@link https://hono.dev/docs/api/request#bytes}
   *
   * @example
   * ```ts
   * app.post('/entry', async (c) => {
   *   const body = await c.req.bytes()
   * })
   * ```
   */
  bytes() {
    return this.#cachedBody("arrayBuffer").then((buffer) => new Uint8Array(buffer));
  }
  /**
   * Parses the request body as a `Blob`.
   * @example
   * ```ts
   * app.post('/entry', async (c) => {
   *   const body = await c.req.blob();
   * });
   * ```
   * @see https://hono.dev/docs/api/request#blob
   */
  blob() {
    return this.#cachedBody("blob");
  }
  /**
   * Parses the request body as `FormData`.
   * @example
   * ```ts
   * app.post('/entry', async (c) => {
   *   const body = await c.req.formData();
   * });
   * ```
   * @see https://hono.dev/docs/api/request#formdata
   */
  formData() {
    return this.#cachedBody("formData");
  }
  /**
   * Adds validated data to the request.
   *
   * @param target - The target of the validation.
   * @param data - The validated data to add.
   */
  addValidatedData(target, data) {
    ;
    (this.#validatedData ??= {})[target] = data;
  }
  valid(target) {
    return this.#validatedData?.[target];
  }
  /**
   * `.url()` can get the request url strings.
   *
   * @see {@link https://hono.dev/docs/api/request#url}
   *
   * @example
   * ```ts
   * app.get('/about/me', (c) => {
   *   const url = c.req.url // `http://localhost:8787/about/me`
   *   ...
   * })
   * ```
   */
  get url() {
    return this.raw.url;
  }
  /**
   * `.method()` can get the method name of the request.
   *
   * @see {@link https://hono.dev/docs/api/request#method}
   *
   * @example
   * ```ts
   * app.get('/about/me', (c) => {
   *   const method = c.req.method // `GET`
   * })
   * ```
   */
  get method() {
    return this.raw.method;
  }
  get [GET_MATCH_RESULT]() {
    return this.#matchResult;
  }
  /**
   * `.matchedRoutes()` can return a matched route in the handler
   *
   * @deprecated
   *
   * Use matchedRoutes helper defined in "hono/route" instead.
   *
   * @see {@link https://hono.dev/docs/api/request#matchedroutes}
   *
   * @example
   * ```ts
   * app.use('*', async function logger(c, next) {
   *   await next()
   *   c.req.matchedRoutes.forEach(({ handler, method, path }, i) => {
   *     const name = handler.name || (handler.length < 2 ? '[handler]' : '[middleware]')
   *     console.log(
   *       method,
   *       ' ',
   *       path,
   *       ' '.repeat(Math.max(10 - path.length, 0)),
   *       name,
   *       i === c.req.routeIndex ? '<- respond from here' : ''
   *     )
   *   })
   * })
   * ```
   */
  get matchedRoutes() {
    return this.#matchResult[0].map(([[, route]]) => route);
  }
  /**
   * `routePath()` can retrieve the path registered within the handler
   *
   * @deprecated
   *
   * Use routePath helper defined in "hono/route" instead.
   *
   * @see {@link https://hono.dev/docs/api/request#routepath}
   *
   * @example
   * ```ts
   * app.get('/posts/:id', (c) => {
   *   return c.json({ path: c.req.routePath })
   * })
   * ```
   */
  get routePath() {
    return this.#matchResult[0].map(([[, route]]) => route)[this.routeIndex].path;
  }
};

// node_modules/hono/dist/utils/html.js
var HtmlEscapedCallbackPhase = {
  Stringify: 1,
  BeforeStream: 2,
  Stream: 3
};
var raw = (value, callbacks) => {
  const escapedString = new String(value);
  escapedString.isEscaped = true;
  escapedString.callbacks = callbacks;
  return escapedString;
};
var resolveCallback = async (str, phase, preserveCallbacks, context, buffer) => {
  if (typeof str === "object" && !(str instanceof String)) {
    if (!(str instanceof Promise)) {
      str = str.toString();
    }
    if (str instanceof Promise) {
      str = await str;
    }
  }
  const callbacks = str.callbacks;
  if (!callbacks?.length) {
    return Promise.resolve(str);
  }
  if (buffer) {
    buffer[0] += str;
  } else {
    buffer = [str];
  }
  const resStr = Promise.all(callbacks.map((c) => c({ phase, buffer, context }))).then(
    (res) => Promise.all(
      res.filter(Boolean).map((str2) => resolveCallback(str2, phase, false, context, buffer))
    ).then(() => buffer[0])
  );
  if (preserveCallbacks) {
    return raw(await resStr, callbacks);
  } else {
    return resStr;
  }
};

// node_modules/hono/dist/context.js
var TEXT_PLAIN = "text/plain; charset=UTF-8";
var setDefaultContentType = (contentType, headers) => {
  return {
    "Content-Type": contentType,
    ...headers
  };
};
var createResponseInstance = (body, init) => new Response(body, init);
var Context = class {
  #rawRequest;
  #req;
  /**
   * `.env` can get bindings (environment variables, secrets, KV namespaces, D1 database, R2 bucket etc.) in Cloudflare Workers.
   *
   * @see {@link https://hono.dev/docs/api/context#env}
   *
   * @example
   * ```ts
   * // Environment object for Cloudflare Workers
   * app.get('*', async c => {
   *   const counter = c.env.COUNTER
   * })
   * ```
   */
  env = {};
  #var;
  finalized = false;
  /**
   * `.error` can get the error object from the middleware if the Handler throws an error.
   *
   * @see {@link https://hono.dev/docs/api/context#error}
   *
   * @example
   * ```ts
   * app.use('*', async (c, next) => {
   *   await next()
   *   if (c.error) {
   *     // do something...
   *   }
   * })
   * ```
   */
  error;
  #status;
  #executionCtx;
  #res;
  #layout;
  #renderer;
  #notFoundHandler;
  #preparedHeaders;
  #matchResult;
  #path;
  /**
   * Creates an instance of the Context class.
   *
   * @param req - The Request object.
   * @param options - Optional configuration options for the context.
   */
  constructor(req, options) {
    this.#rawRequest = req;
    if (options) {
      this.#executionCtx = options.executionCtx;
      this.env = options.env;
      this.#notFoundHandler = options.notFoundHandler;
      this.#path = options.path;
      this.#matchResult = options.matchResult;
    }
  }
  /**
   * `.req` is the instance of {@link HonoRequest}.
   */
  get req() {
    this.#req ??= new HonoRequest(this.#rawRequest, this.#path, this.#matchResult);
    return this.#req;
  }
  /**
   * @see {@link https://hono.dev/docs/api/context#event}
   * The FetchEvent associated with the current request.
   *
   * @throws Will throw an error if the context does not have a FetchEvent.
   */
  get event() {
    if (this.#executionCtx && "respondWith" in this.#executionCtx) {
      return this.#executionCtx;
    } else {
      throw Error("This context has no FetchEvent");
    }
  }
  /**
   * @see {@link https://hono.dev/docs/api/context#executionctx}
   * The ExecutionContext associated with the current request.
   *
   * @throws Will throw an error if the context does not have an ExecutionContext.
   */
  get executionCtx() {
    if (this.#executionCtx) {
      return this.#executionCtx;
    } else {
      throw Error("This context has no ExecutionContext");
    }
  }
  /**
   * @see {@link https://hono.dev/docs/api/context#res}
   * The Response object for the current request.
   */
  get res() {
    return this.#res ||= createResponseInstance(null, {
      headers: this.#preparedHeaders ??= new Headers()
    });
  }
  /**
   * Sets the Response object for the current request.
   *
   * @param _res - The Response object to set.
   */
  set res(_res) {
    if (this.#res && _res) {
      _res = createResponseInstance(_res.body, _res);
      for (const [k, v] of this.#res.headers.entries()) {
        if (k === "content-type") {
          continue;
        }
        if (k === "set-cookie") {
          const cookies = this.#res.headers.getSetCookie();
          _res.headers.delete("set-cookie");
          for (const cookie of cookies) {
            _res.headers.append("set-cookie", cookie);
          }
        } else {
          _res.headers.set(k, v);
        }
      }
    }
    this.#res = _res;
    this.finalized = true;
  }
  /**
   * `.render()` can create a response within a layout.
   *
   * @see {@link https://hono.dev/docs/api/context#render-setrenderer}
   *
   * @example
   * ```ts
   * app.get('/', (c) => {
   *   return c.render('Hello!')
   * })
   * ```
   */
  render = (...args) => {
    this.#renderer ??= (content) => this.html(content);
    return this.#renderer(...args);
  };
  /**
   * Sets the layout for the response.
   *
   * @param layout - The layout to set.
   * @returns The layout function.
   */
  setLayout = (layout) => this.#layout = layout;
  /**
   * Gets the current layout for the response.
   *
   * @returns The current layout function.
   */
  getLayout = () => this.#layout;
  /**
   * `.setRenderer()` can set the layout in the custom middleware.
   *
   * @see {@link https://hono.dev/docs/api/context#render-setrenderer}
   *
   * @example
   * ```tsx
   * app.use('*', async (c, next) => {
   *   c.setRenderer((content) => {
   *     return c.html(
   *       <html>
   *         <body>
   *           <p>{content}</p>
   *         </body>
   *       </html>
   *     )
   *   })
   *   await next()
   * })
   * ```
   */
  setRenderer = (renderer) => {
    this.#renderer = renderer;
  };
  /**
   * `.header()` can set headers.
   *
   * @see {@link https://hono.dev/docs/api/context#header}
   *
   * @example
   * ```ts
   * app.get('/welcome', (c) => {
   *   // Set headers
   *   c.header('X-Message', 'Hello!')
   *   c.header('Content-Type', 'text/plain')
   *
   *   // Append multiple headers using the append option (e.g. Vary)
   *   c.header('Vary', 'Accept-Encoding', { append: true })
   *   c.header('Vary', 'User-Agent', { append: true })
   *
   *   return c.body('Thank you for coming')
   * })
   * ```
   */
  header = (name, value, options) => {
    if (this.finalized) {
      this.#res = createResponseInstance(this.#res.body, this.#res);
    }
    const headers = this.#res ? this.#res.headers : this.#preparedHeaders ??= new Headers();
    if (value === void 0) {
      headers.delete(name);
    } else if (options?.append) {
      headers.append(name, value);
    } else {
      headers.set(name, value);
    }
  };
  status = (status) => {
    this.#status = status;
  };
  /**
   * `.set()` can set the value specified by the key.
   *
   * @see {@link https://hono.dev/docs/api/context#set-get}
   *
   * @example
   * ```ts
   * app.use('*', async (c, next) => {
   *   c.set('message', 'Hono is hot!!')
   *   await next()
   * })
   * ```
   */
  set = (key, value) => {
    this.#var ??= /* @__PURE__ */ new Map();
    this.#var.set(key, value);
  };
  /**
   * `.get()` can use the value specified by the key.
   *
   * @see {@link https://hono.dev/docs/api/context#set-get}
   *
   * @example
   * ```ts
   * app.get('/', (c) => {
   *   const message = c.get('message')
   *   return c.text(`The message is "${message}"`)
   * })
   * ```
   */
  get = (key) => {
    return this.#var ? this.#var.get(key) : void 0;
  };
  /**
   * `.var` can access the value of a variable.
   *
   * @see {@link https://hono.dev/docs/api/context#var}
   *
   * @example
   * ```ts
   * const result = c.var.client.oneMethod()
   * ```
   */
  // c.var.propName is a read-only
  get var() {
    if (!this.#var) {
      return {};
    }
    return Object.fromEntries(this.#var);
  }
  #newResponse(data, arg, headers) {
    let responseHeaders2 = this.#res ? new Headers(this.#res.headers) : this.#preparedHeaders;
    if (typeof arg === "object" && arg.headers) {
      responseHeaders2 ??= new Headers();
      for (const [key, value] of new Headers(arg.headers)) {
        if (key === "set-cookie") {
          responseHeaders2.append(key, value);
        } else {
          responseHeaders2.set(key, value);
        }
      }
    }
    if (headers) {
      if (!responseHeaders2) {
        let count = 0;
        for (const k in headers) {
          if (++count > 1 || typeof headers[k] !== "string") {
            responseHeaders2 = new Headers();
            break;
          }
        }
      }
      if (responseHeaders2) {
        for (const k in headers) {
          const v = headers[k];
          if (typeof v === "string") {
            responseHeaders2.set(k, v);
          } else {
            responseHeaders2.delete(k);
            for (const v2 of v) {
              responseHeaders2.append(k, v2);
            }
          }
        }
      }
    }
    const status = typeof arg === "number" ? arg : arg?.status ?? this.#status;
    return createResponseInstance(data, {
      status,
      headers: responseHeaders2 ?? headers
    });
  }
  newResponse = (...args) => this.#newResponse(...args);
  /**
   * `.body()` can return the HTTP response.
   * You can set headers with `.header()` and set HTTP status code with `.status`.
   * This can also be set in `.text()`, `.json()` and so on.
   *
   * @see {@link https://hono.dev/docs/api/context#body}
   *
   * @example
   * ```ts
   * app.get('/welcome', (c) => {
   *   // Set headers
   *   c.header('X-Message', 'Hello!')
   *   c.header('Content-Type', 'text/plain')
   *   // Set HTTP status code
   *   c.status(201)
   *
   *   // Return the response body
   *   return c.body('Thank you for coming')
   * })
   * ```
   */
  body = (data, arg, headers) => this.#newResponse(data, arg, headers);
  /**
   * `.text()` can render text as `Content-Type:text/plain`.
   *
   * @see {@link https://hono.dev/docs/api/context#text}
   *
   * @example
   * ```ts
   * app.get('/say', (c) => {
   *   return c.text('Hello!')
   * })
   * ```
   */
  text = (text, arg, headers) => {
    return !this.#preparedHeaders && !this.#status && !arg && !headers && !this.finalized ? new Response(text) : this.#newResponse(
      text,
      arg,
      setDefaultContentType(TEXT_PLAIN, headers)
    );
  };
  /**
   * `.json()` can render JSON as `Content-Type:application/json`.
   *
   * @see {@link https://hono.dev/docs/api/context#json}
   *
   * @example
   * ```ts
   * app.get('/api', (c) => {
   *   return c.json({ message: 'Hello!' })
   * })
   * ```
   */
  json = (object, arg, headers) => {
    return this.#newResponse(
      JSON.stringify(object),
      arg,
      setDefaultContentType("application/json", headers)
    );
  };
  html = (html, arg, headers) => {
    const res = (html2) => this.#newResponse(html2, arg, setDefaultContentType("text/html; charset=UTF-8", headers));
    return typeof html === "object" ? resolveCallback(html, HtmlEscapedCallbackPhase.Stringify, false, {}).then(res) : res(html);
  };
  /**
   * `.redirect()` can Redirect, default status code is 302.
   *
   * @see {@link https://hono.dev/docs/api/context#redirect}
   *
   * @example
   * ```ts
   * app.get('/redirect', (c) => {
   *   return c.redirect('/')
   * })
   * app.get('/redirect-permanently', (c) => {
   *   return c.redirect('/', 301)
   * })
   * ```
   */
  redirect = (location, status) => {
    const locationString = String(location);
    this.header(
      "Location",
      // Multibytes should be encoded
      // eslint-disable-next-line no-control-regex
      !/[^\x00-\xFF]/.test(locationString) ? locationString : encodeURI(locationString)
    );
    return this.newResponse(null, status ?? 302);
  };
  /**
   * `.notFound()` can return the Not Found Response.
   *
   * @see {@link https://hono.dev/docs/api/context#notfound}
   *
   * @example
   * ```ts
   * app.get('/notfound', (c) => {
   *   return c.notFound()
   * })
   * ```
   */
  notFound = () => {
    this.#notFoundHandler ??= () => createResponseInstance();
    return this.#notFoundHandler(this);
  };
};

// node_modules/hono/dist/router.js
var METHOD_NAME_ALL = "ALL";
var METHOD_NAME_ALL_LOWERCASE = "all";
var METHODS = ["get", "post", "put", "delete", "options", "patch", "query"];
var MESSAGE_MATCHER_IS_ALREADY_BUILT = "Can not add a route since the matcher is already built.";
var UnsupportedPathError = class extends Error {
};

// node_modules/hono/dist/utils/constants.js
var COMPOSED_HANDLER = "__COMPOSED_HANDLER";

// node_modules/hono/dist/hono-base.js
var notFoundHandler = (c) => {
  return c.text("404 Not Found", 404);
};
var errorHandler = (err, c) => {
  if ("getResponse" in err) {
    const res = err.getResponse();
    return c.newResponse(res.body, res);
  }
  console.error(err);
  return c.text("Internal Server Error", 500);
};
var Hono = class _Hono {
  get;
  post;
  put;
  delete;
  options;
  patch;
  query;
  all;
  on;
  use;
  /*
    This class is like an abstract class and does not have a router.
    To use it, inherit the class and implement router in the constructor.
  */
  router;
  getPath;
  // Cannot use `#` because it requires visibility at JavaScript runtime.
  _basePath = "/";
  #path = "/";
  routes = [];
  constructor(options = {}) {
    const allMethods = [...METHODS, METHOD_NAME_ALL_LOWERCASE];
    allMethods.forEach((method) => {
      this[method] = (args1, ...args) => {
        const methodName = method.toUpperCase();
        if (typeof args1 === "string") {
          this.#path = args1;
        } else {
          this.#addRoute(methodName, this.#path, args1);
        }
        args.forEach((handler) => {
          this.#addRoute(methodName, this.#path, handler);
        });
        return this;
      };
    });
    this.on = (method, path, ...handlers) => {
      for (const p of [path].flat()) {
        this.#path = p;
        for (const m of [method].flat()) {
          const methodName = m.toUpperCase();
          for (const handler of handlers) {
            this.#addRoute(methodName, this.#path, handler);
          }
        }
      }
      return this;
    };
    this.use = (arg1, ...handlers) => {
      if (typeof arg1 === "string") {
        this.#path = arg1;
      } else {
        this.#path = "*";
        handlers.unshift(arg1);
      }
      handlers.forEach((handler) => {
        this.#addRoute(METHOD_NAME_ALL, this.#path, handler);
      });
      return this;
    };
    const { strict, ...optionsWithoutStrict } = options;
    Object.assign(this, optionsWithoutStrict);
    this.getPath = strict ?? true ? options.getPath ?? getPath : getPathNoStrict;
  }
  #clone() {
    const clone = new _Hono({
      router: this.router,
      getPath: this.getPath
    });
    clone.errorHandler = this.errorHandler;
    clone.#notFoundHandler = this.#notFoundHandler;
    clone.routes = this.routes;
    return clone;
  }
  #notFoundHandler = notFoundHandler;
  // Cannot use `#` because it requires visibility at JavaScript runtime.
  errorHandler = errorHandler;
  /**
   * `.route()` allows grouping other Hono instance in routes.
   *
   * @see {@link https://hono.dev/docs/api/routing#grouping}
   *
   * @param {string} path - base Path
   * @param {Hono} app - other Hono instance
   * @returns {Hono} routed Hono instance
   *
   * @example
   * ```ts
   * const app = new Hono()
   * const app2 = new Hono()
   *
   * app2.get("/user", (c) => c.text("user"))
   * app.route("/api", app2) // GET /api/user
   * ```
   */
  route(path, app2) {
    const subApp = this.basePath(path);
    app2.routes.map((r) => {
      let handler;
      if (app2.errorHandler === errorHandler) {
        handler = r.handler;
      } else {
        handler = async (c, next) => (await compose([], app2.errorHandler)(c, () => r.handler(c, next))).res;
        handler[COMPOSED_HANDLER] = r.handler;
      }
      subApp.#addRoute(r.method, r.path, handler, r.basePath);
    });
    return this;
  }
  /**
   * `.basePath()` allows base paths to be specified.
   *
   * @see {@link https://hono.dev/docs/api/routing#base-path}
   *
   * @param {string} path - base Path
   * @returns {Hono} changed Hono instance
   *
   * @example
   * ```ts
   * const api = new Hono().basePath('/api')
   * ```
   */
  basePath(path) {
    const subApp = this.#clone();
    subApp._basePath = mergePath(this._basePath, path);
    return subApp;
  }
  /**
   * `.onError()` handles an error and returns a customized Response.
   *
   * @see {@link https://hono.dev/docs/api/hono#error-handling}
   *
   * @param {ErrorHandler} handler - request Handler for error
   * @returns {Hono} changed Hono instance
   *
   * @example
   * ```ts
   * app.onError((err, c) => {
   *   console.error(`${err}`)
   *   return c.text('Custom Error Message', 500)
   * })
   * ```
   */
  onError = (handler) => {
    this.errorHandler = handler;
    return this;
  };
  /**
   * `.notFound()` allows you to customize a Not Found Response.
   *
   * @see {@link https://hono.dev/docs/api/hono#not-found}
   *
   * @param {NotFoundHandler} handler - request handler for not-found
   * @returns {Hono} changed Hono instance
   *
   * @example
   * ```ts
   * app.notFound((c) => {
   *   return c.text('Custom 404 Message', 404)
   * })
   * ```
   */
  notFound = (handler) => {
    this.#notFoundHandler = handler;
    return this;
  };
  /**
   * `.mount()` allows you to mount applications built with other frameworks into your Hono application.
   *
   * @see {@link https://hono.dev/docs/api/hono#mount}
   *
   * @param {string} path - base Path
   * @param {Function} applicationHandler - other Request Handler
   * @param {MountOptions} [options] - options of `.mount()`
   * @returns {Hono} mounted Hono instance
   *
   * @example
   * ```ts
   * import { Router as IttyRouter } from 'itty-router'
   * import { Hono } from 'hono'
   * // Create itty-router application
   * const ittyRouter = IttyRouter()
   * // GET /itty-router/hello
   * ittyRouter.get('/hello', () => new Response('Hello from itty-router'))
   *
   * const app = new Hono()
   * app.mount('/itty-router', ittyRouter.handle)
   * ```
   *
   * @example
   * ```ts
   * const app = new Hono()
   * // Send the request to another application without modification.
   * app.mount('/app', anotherApp, {
   *   replaceRequest: (req) => req,
   * })
   * ```
   */
  mount(path, applicationHandler, options) {
    let replaceRequest;
    let optionHandler;
    if (options) {
      if (typeof options === "function") {
        optionHandler = options;
      } else {
        optionHandler = options.optionHandler;
        if (options.replaceRequest === false) {
          replaceRequest = (request) => request;
        } else {
          replaceRequest = options.replaceRequest;
        }
      }
    }
    const getOptions = optionHandler ? (c) => {
      const options2 = optionHandler(c);
      return Array.isArray(options2) ? options2 : [options2];
    } : (c) => {
      let executionContext = void 0;
      try {
        executionContext = c.executionCtx;
      } catch {
      }
      return [c.env, executionContext];
    };
    replaceRequest ||= (() => {
      const mergedPath = mergePath(this._basePath, path);
      const pathPrefixLength = mergedPath === "/" ? 0 : mergedPath.length;
      return (request) => {
        const url = new URL(request.url);
        url.pathname = this.getPath(request).slice(pathPrefixLength) || "/";
        return new Request(url, request);
      };
    })();
    const handler = async (c, next) => {
      const res = await applicationHandler(replaceRequest(c.req.raw), ...getOptions(c));
      if (res) {
        return res;
      }
      await next();
    };
    this.#addRoute(METHOD_NAME_ALL, mergePath(path, "*"), handler);
    return this;
  }
  #addRoute(method, path, handler, baseRoutePath) {
    path = mergePath(this._basePath, path);
    const r = {
      basePath: baseRoutePath !== void 0 ? mergePath(this._basePath, baseRoutePath) : this._basePath,
      path,
      method,
      handler
    };
    this.router.add(method, path, [handler, r]);
    this.routes.push(r);
  }
  #handleError(err, c) {
    if (err instanceof Error) {
      return this.errorHandler(err, c);
    }
    throw err;
  }
  #dispatch(request, executionCtx, env, method) {
    if (method === "HEAD") {
      return (async () => new Response(null, await this.#dispatch(request, executionCtx, env, "GET")))();
    }
    const path = this.getPath(request, { env });
    const matchResult = this.router.match(method, path);
    const c = new Context(request, {
      path,
      matchResult,
      env,
      executionCtx,
      notFoundHandler: this.#notFoundHandler
    });
    if (matchResult[0].length === 1) {
      let res;
      try {
        res = matchResult[0][0][0][0](c, async () => {
          c.res = await this.#notFoundHandler(c);
        });
      } catch (err) {
        return this.#handleError(err, c);
      }
      return res instanceof Promise ? res.then(
        (resolved) => resolved || (c.finalized ? c.res : this.#notFoundHandler(c))
      ).catch((err) => this.#handleError(err, c)) : res ?? this.#notFoundHandler(c);
    }
    const composed = compose(matchResult[0], this.errorHandler, this.#notFoundHandler);
    return (async () => {
      try {
        const context = await composed(c);
        if (!context.finalized) {
          throw new Error(
            "Context is not finalized. Did you forget to return a Response object or `await next()`?"
          );
        }
        return context.res;
      } catch (err) {
        return this.#handleError(err, c);
      }
    })();
  }
  /**
   * `.fetch()` will be entry point of your app.
   *
   * @see {@link https://hono.dev/docs/api/hono#fetch}
   *
   * @param {Request} request - request Object of request
   * @param {Env} env - env Object
   * @param {ExecutionContext} executionCtx - context of execution
   * @returns {Response | Promise<Response>} response of request
   *
   */
  fetch = (request, ...rest) => {
    return this.#dispatch(request, rest[1], rest[0], request.method);
  };
  /**
   * `.request()` is a useful method for testing.
   * You can pass a URL or pathname to send a GET request.
   * app will return a Response object.
   * ```ts
   * test('GET /hello is ok', async () => {
   *   const res = await app.request('/hello')
   *   expect(res.status).toBe(200)
   * })
   * ```
   * @see https://hono.dev/docs/api/hono#request
   */
  request = (input, requestInit, Env, executionCtx) => {
    if (input instanceof Request) {
      return this.fetch(requestInit ? new Request(input, requestInit) : input, Env, executionCtx);
    }
    input = input.toString();
    return this.fetch(
      new Request(
        /^https?:\/\//.test(input) ? input : `http://localhost${mergePath("/", input)}`,
        requestInit
      ),
      Env,
      executionCtx
    );
  };
  /**
   * `.fire()` automatically adds a global fetch event listener.
   * This can be useful for environments that adhere to the Service Worker API, such as non-ES module Cloudflare Workers.
   * @deprecated
   * Use `fire` from `hono/service-worker` instead.
   * ```ts
   * import { Hono } from 'hono'
   * import { fire } from 'hono/service-worker'
   *
   * const app = new Hono()
   * // ...
   * fire(app)
   * ```
   * @see https://hono.dev/docs/api/hono#fire
   * @see https://developer.mozilla.org/en-US/docs/Web/API/Service_Worker_API
   * @see https://developers.cloudflare.com/workers/reference/migrate-to-module-workers/
   */
  fire = () => {
    addEventListener("fetch", (event) => {
      event.respondWith(this.#dispatch(event.request, event, void 0, event.request.method));
    });
  };
};

// node_modules/hono/dist/router/linear-router/router.js
var emptyParams = /* @__PURE__ */ Object.create(null);
var splitPathRe = /\/(:\w+(?:{(?:(?:{[\d,]+})|[^}])+})?)|\/[^\/\?]+|(\?)/g;
var splitByStarRe = /\*/;
var LinearRouter = class {
  name = "LinearRouter";
  #routes = [];
  add(method, path, handler) {
    for (let i = 0, paths = checkOptionalParameter(path) || [path], len = paths.length; i < len; i++) {
      this.#routes.push([method, paths[i], handler]);
    }
  }
  match(method, path) {
    const handlers = [];
    ROUTES_LOOP: for (let i = 0, len = this.#routes.length; i < len; i++) {
      const [routeMethod, routePath, handler] = this.#routes[i];
      if (routeMethod === method || routeMethod === METHOD_NAME_ALL) {
        if (routePath === "*" || routePath === "/*") {
          handlers.push([handler, emptyParams]);
          continue;
        }
        const hasStar = routePath.indexOf("*") !== -1;
        const hasLabel = routePath.indexOf(":") !== -1;
        if (!hasStar && !hasLabel) {
          if (routePath === path || routePath + "/" === path) {
            handlers.push([handler, emptyParams]);
          }
        } else if (hasStar && !hasLabel) {
          const endsWithStar = routePath.charCodeAt(routePath.length - 1) === 42;
          const endsWithSlashStar = routePath.endsWith("/*");
          const parts = (endsWithStar ? routePath.slice(0, endsWithSlashStar ? -2 : -1) : routePath).split(splitByStarRe);
          const lastIndex = parts.length - 1;
          for (let j = 0, pos = 0, len2 = parts.length; j < len2; j++) {
            const part = parts[j];
            const index = path.indexOf(part, pos);
            if (index !== pos) {
              continue ROUTES_LOOP;
            }
            pos += part.length;
            if (j === lastIndex) {
              if (endsWithSlashStar) {
                if (pos !== path.length && path.charCodeAt(pos) !== 47) {
                  continue ROUTES_LOOP;
                }
              } else if (!endsWithStar && pos !== path.length && !(pos === path.length - 1 && path.charCodeAt(pos) === 47)) {
                continue ROUTES_LOOP;
              }
            } else {
              const index2 = path.indexOf("/", pos);
              if (index2 === -1) {
                continue ROUTES_LOOP;
              }
              pos = index2;
            }
          }
          handlers.push([handler, emptyParams]);
        } else if (hasLabel && !hasStar) {
          const params = /* @__PURE__ */ Object.create(null);
          const parts = routePath.match(splitPathRe);
          const lastIndex = parts.length - 1;
          for (let j = 0, pos = 0, len2 = parts.length; j < len2; j++) {
            if (pos === -1 || pos >= path.length) {
              continue ROUTES_LOOP;
            }
            const part = parts[j];
            if (part.charCodeAt(1) === 58) {
              if (path.charCodeAt(pos) !== 47) {
                continue ROUTES_LOOP;
              }
              let name = part.slice(2);
              let value;
              if (name.charCodeAt(name.length - 1) === 125) {
                const openBracePos = name.indexOf("{");
                const next = parts[j + 1];
                const lookahead = next && next[1] !== ":" && next[1] !== "*" ? `(?=${next})` : "";
                const pattern = name.slice(openBracePos + 1, -1) + lookahead;
                const restPath = path.slice(pos + 1);
                const match = new RegExp(pattern, "d").exec(restPath);
                if (!match || match.indices[0][0] !== 0 || match.indices[0][1] === 0) {
                  continue ROUTES_LOOP;
                }
                name = name.slice(0, openBracePos);
                value = restPath.slice(...match.indices[0]);
                pos += match.indices[0][1] + 1;
              } else {
                let endValuePos = path.indexOf("/", pos + 1);
                if (endValuePos === -1) {
                  if (pos + 1 === path.length) {
                    continue ROUTES_LOOP;
                  }
                  endValuePos = path.length;
                }
                value = path.slice(pos + 1, endValuePos);
                pos = endValuePos;
              }
              params[name] ||= value;
            } else {
              const index = path.indexOf(part, pos);
              if (index !== pos) {
                continue ROUTES_LOOP;
              }
              pos += part.length;
            }
            if (j === lastIndex) {
              if (pos !== path.length && !(pos === path.length - 1 && path.charCodeAt(pos) === 47)) {
                continue ROUTES_LOOP;
              }
            }
          }
          handlers.push([handler, params]);
        } else if (hasLabel && hasStar) {
          throw new UnsupportedPathError();
        }
      }
    }
    return [handlers];
  }
};

// node_modules/hono/dist/router/smart-router/router.js
var SmartRouter = class {
  name = "SmartRouter";
  #routers = [];
  #routes = [];
  constructor(init) {
    this.#routers = init.routers;
  }
  add(method, path, handler) {
    if (!this.#routes) {
      throw new Error(MESSAGE_MATCHER_IS_ALREADY_BUILT);
    }
    this.#routes.push([method, path, handler]);
  }
  match(method, path) {
    if (!this.#routes) {
      throw new Error("Fatal error");
    }
    const routers = this.#routers;
    const routes = this.#routes;
    const len = routers.length;
    let i = 0;
    let res;
    for (; i < len; i++) {
      const router = routers[i];
      try {
        for (let i2 = 0, len2 = routes.length; i2 < len2; i2++) {
          router.add(...routes[i2]);
        }
        res = router.match(method, path);
      } catch (e) {
        if (e instanceof UnsupportedPathError) {
          continue;
        }
        throw e;
      }
      this.match = router.match.bind(router);
      this.#routers = [router];
      this.#routes = void 0;
      break;
    }
    if (i === len) {
      throw new Error("Fatal error");
    }
    this.name = `SmartRouter + ${this.activeRouter.name}`;
    return res;
  }
  get activeRouter() {
    if (this.#routes || this.#routers.length !== 1) {
      throw new Error("No active router has been determined yet.");
    }
    return this.#routers[0];
  }
};

// node_modules/hono/dist/router/utils.js
var createNullObject = () => /* @__PURE__ */ Object.create(null);

// node_modules/hono/dist/router/trie-router/node.js
var emptyParams2 = createNullObject();
var order = 0;
var Node = class _Node {
  #methods = [];
  #children = createNullObject();
  #patterns = [];
  #pattern;
  #params = emptyParams2;
  insert(method, path, handler) {
    let curNode = this;
    const parts = splitRoutingPath(path);
    const possibleKeys = /* @__PURE__ */ new Set();
    let i = 0;
    for (const p of parts) {
      const nextP = parts[++i];
      const pattern = getPattern(p, nextP) || (nextP === void 0 && p && p.indexOf("*") === p.length - 1 ? p : null);
      const isParam = Array.isArray(pattern);
      const key = isParam ? pattern[0] : pattern || p;
      const child = curNode.#children[key] ||= new _Node();
      if (pattern && !child.#pattern) {
        child.#pattern = pattern;
        curNode.#patterns.push(child);
      }
      curNode = child;
      if (isParam) {
        possibleKeys.add(pattern[1]);
      }
    }
    curNode.#methods.push({
      [method]: {
        handler,
        possibleKeys: [...possibleKeys],
        score: ++order
      }
    });
  }
  #pushHandlerSets(handlerSets, node, method, nodeParams, params) {
    for (let i = 0, len = node.#methods.length; i < len; i++) {
      const m = node.#methods[i];
      const handlerSet = m[method] || m[METHOD_NAME_ALL];
      if (handlerSet) {
        handlerSet.params = createNullObject();
        handlerSets.push(handlerSet);
        for (let i2 = 0, len2 = handlerSet.possibleKeys.length; i2 < len2; i2++) {
          const key = handlerSet.possibleKeys[i2];
          handlerSet.params[key] = params?.[key] && !i2 ? params[key] : nodeParams[key] ?? params?.[key];
        }
      }
    }
  }
  search(method, path) {
    const handlerSets = [];
    this.#params = emptyParams2;
    const curNode = this;
    let curNodes = [curNode];
    const parts = splitPath(path);
    const curNodesQueue = [];
    const len = parts.length;
    let partOffsets = null;
    for (let i = 0; i < len; i++) {
      const part = parts[i];
      const isLast = i === len - 1;
      const tempNodes = [];
      for (let j = 0, len2 = curNodes.length; j < len2; j++) {
        const node = curNodes[j];
        const nextNode = node.#children[part];
        if (nextNode) {
          nextNode.#params = node.#params;
          if (isLast) {
            if (nextNode.#children["*"]) {
              this.#pushHandlerSets(handlerSets, nextNode.#children["*"], method, node.#params);
            }
            this.#pushHandlerSets(handlerSets, nextNode, method, node.#params);
          } else {
            tempNodes.push(nextNode);
          }
        }
        for (const child of node.#patterns) {
          const pattern = child.#pattern;
          const params = node.#params === emptyParams2 ? {} : { ...node.#params };
          if (typeof pattern === "string") {
            if (pattern === "*" || part.startsWith(pattern.slice(0, -1))) {
              this.#pushHandlerSets(handlerSets, child, method, node.#params);
              if (pattern === "*") {
                child.#params = params;
                tempNodes.push(child);
              }
            }
            continue;
          }
          const [, name, matcher] = pattern;
          if (!part && matcher === true) {
            continue;
          }
          if (matcher !== true) {
            if (!partOffsets) {
              partOffsets = [];
              let offset = path[0] === "/" ? 1 : 0;
              for (let p = 0; p < len; p++) {
                partOffsets[p] = offset;
                offset += parts[p].length + 1;
              }
            }
            const restPathString = path.slice(partOffsets[i]);
            const m = matcher.exec(restPathString);
            if (m) {
              params[name] = m[0];
              this.#pushHandlerSets(handlerSets, child, method, node.#params, params);
              if (m[0].length === restPathString.length && child.#children["*"]) {
                this.#pushHandlerSets(
                  handlerSets,
                  child.#children["*"],
                  method,
                  node.#params,
                  params
                );
              }
              for (const _ in child.#children) {
                child.#params = params;
                const componentCount = m[0].match(/\//g)?.length ?? 0;
                const targetCurNodes = curNodesQueue[componentCount] ||= [];
                targetCurNodes.push(child);
                break;
              }
              continue;
            }
          }
          if (matcher === true || matcher.test(part)) {
            params[name] = part;
            if (isLast) {
              this.#pushHandlerSets(handlerSets, child, method, params, node.#params);
              if (child.#children["*"]) {
                this.#pushHandlerSets(
                  handlerSets,
                  child.#children["*"],
                  method,
                  params,
                  node.#params
                );
              }
            } else {
              child.#params = params;
              tempNodes.push(child);
            }
          }
        }
      }
      const shifted = curNodesQueue.shift();
      curNodes = shifted ? tempNodes.concat(shifted) : tempNodes;
    }
    if (handlerSets[1]) {
      handlerSets.sort((a, b) => {
        return a.score - b.score;
      });
    }
    return [handlerSets.map(({ handler, params }) => [handler, params])];
  }
};

// node_modules/hono/dist/router/trie-router/router.js
var TrieRouter = class {
  name = "TrieRouter";
  #node = new Node();
  add(method, path, handler) {
    for (const result of checkOptionalParameter(path) || [path]) {
      this.#node.insert(method, result, handler);
    }
  }
  match(method, path) {
    return this.#node.search(method, path);
  }
};

// node_modules/hono/dist/preset/quick.js
var Hono2 = class extends Hono {
  constructor(options = {}) {
    super(options);
    this.router = new SmartRouter({
      routers: [new LinearRouter(), new TrieRouter()]
    });
  }
};

// src/auth.js
import { DurableObject } from "cloudflare:workers";
var CLIENT_ID = "app_EMoamEEZ73f0CkXaXp7hrann";
var credential = (env, id) => env.CREDS.get(env.CREDS.idFromName(id));
var directory = (env) => env.POOL.get(env.POOL.idFromName("directory"));
var account = (s) => ({
  id: s.id,
  state: s.state,
  expiresAt: s.expiresAt,
  accountId: s.accountId,
  cooldown: s.cooldown ?? 0,
  enabled: s.enabled !== false
});
var authBase = (env) => env.AUTH_BASE ?? "https://auth.openai.com";
function jwt(token) {
  if (!token) return {};
  const segment = token.split(".")[1];
  if (!segment) throw new Error("invalid id_token");
  const base64 = segment.replace(/-/g, "+").replace(/_/g, "/");
  const bytes = Uint8Array.from(
    atob(base64 + "=".repeat((4 - base64.length % 4) % 4)),
    (c) => c.charCodeAt(0)
  );
  return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
}
function tokenState(id, old, data) {
  if (!data.access_token) throw new Error("OAuth response lacks access_token");
  const claims = jwt(
    data.id_token ?? (data.access_token?.split(".").length === 3 ? data.access_token : void 0)
  );
  const expires = Number(data.expires_in ?? 3600);
  if (!Number.isFinite(expires) || expires < 0)
    throw new Error("invalid token expiry");
  return {
    ...old,
    id,
    state: "active",
    access: data.access_token,
    refresh: data.refresh_token ?? old?.refresh,
    accountId: claims["https://api.openai.com/auth"]?.chatgpt_account_id ?? claims.chatgpt_account_id ?? old?.accountId,
    expiresAt: Date.now() + expires * 1e3,
    cooldown: 0
  };
}
var Credential = class extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    ctx.blockConcurrencyWhile(async () => {
      this.state = await ctx.storage.get("state");
    });
  }
  async install(state) {
    await this.ctx.storage.put("state", state);
    this.state = state;
    return account(state);
  }
  async importState(state) {
    if (this.refreshing) await this.refreshing.catch(() => {
    });
    return this.install(state);
  }
  async status() {
    return this.state ? account(this.state) : null;
  }
  async cooldown(seconds) {
    const state = { ...this.state, cooldown: Date.now() + seconds * 1e3 };
    await this.ctx.storage.put("state", state);
    this.state = state;
  }
  async setEnabled(enabled) {
    if (this.refreshing) await this.refreshing.catch(() => {
    });
    if (!this.state) throw new Error("account unavailable");
    return this.install({ ...this.state, enabled });
  }
  async token(force = false, rejectedAccess) {
    if (!this.state || this.state.state === "dead" || this.state.enabled === false)
      throw new Error("account unavailable");
    if (this.state.cooldown > Date.now())
      throw new Error("account cooling down");
    if (force && rejectedAccess && this.state.access !== rejectedAccess)
      return this.state;
    if (!force && this.state.expiresAt > Date.now() + 6e4) return this.state;
    if (!this.refreshing)
      this.refreshing = this.refresh().finally(() => {
        this.refreshing = null;
      });
    return this.refreshing;
  }
  async refresh() {
    if (!this.state.refresh) throw new Error("account requires a new login");
    const response = await this.ctx.fetch(authBase(this.env) + "/oauth/token", {
      method: "POST",
      proxy: this.state.proxy ?? "direct",
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams({
        grant_type: "refresh_token",
        client_id: CLIENT_ID,
        refresh_token: this.state.refresh
      })
    });
    const data = await response.json();
    if (!response.ok) {
      if (["invalid_grant", "refresh_token_reused"].includes(data.error) || ["invalid_grant", "refresh_token_reused"].includes(data.error?.code))
        await this.install({ ...this.state, state: "dead" });
      throw new Error(
        `refresh failed: ${data.error?.code ?? data.error ?? response.status}`
      );
    }
    const state = tokenState(this.state.id, this.state, data);
    await this.install(state);
    return state;
  }
  async maintain() {
    if (!this.state || this.state.state === "dead" || this.state.enabled === false || this.state.cooldown > Date.now())
      return;
    await this.token();
  }
};
var Pool = class extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.polling = /* @__PURE__ */ new Map();
    this.registering = Promise.resolve();
  }
  async register(id) {
    const update = this.registering.then(async () => {
      const ids = await this.ctx.storage.get("accounts") ?? [];
      if (!ids.includes(id)) ids.push(id);
      await this.ctx.storage.put("accounts", ids);
      await this.env.CONFIG.put("accounts", JSON.stringify(ids));
      return ids;
    });
    this.registering = update.catch(() => {
    });
    return update;
  }
  claimUsage(id) {
    const work = this.registering.then(async () => {
      const key = "usage:" + id;
      if (await this.env.CONFIG.get(key)) return false;
      await this.env.CONFIG.put(key, "1", { expirationTtl: 3600 });
      return true;
    });
    this.registering = work.catch(() => {
    });
    return work;
  }
  async pruneLogins() {
    const work = this.registering.then(async () => {
      const ids = await this.ctx.storage.get("logins") ?? [];
      const kept = [];
      for (const id of ids) {
        const value = await this.ctx.storage.get("login:" + id);
        if (!value || value.expires_at <= Date.now())
          await this.ctx.storage.delete("login:" + id);
        else kept.push(id);
      }
      await this.ctx.storage.put("logins", kept);
    });
    this.registering = work.catch(() => {
    });
    return work;
  }
  async candidates(preferred) {
    const ids = await this.ctx.storage.get("accounts") ?? [];
    const cursor = await this.ctx.storage.get("cursor") ?? 0;
    if (!ids.length) return [];
    const start = cursor % ids.length;
    await this.ctx.storage.put("cursor", (start + 1) % ids.length);
    const rotated = [...ids.slice(start), ...ids.slice(0, start)];
    return preferred && ids.includes(preferred) ? [preferred, ...rotated.filter((id) => id !== preferred)] : rotated;
  }
  async login(proxy = "direct") {
    const response = await this.ctx.fetch(
      authBase(this.env) + "/api/accounts/deviceauth/usercode",
      {
        method: "POST",
        proxy,
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ client_id: CLIENT_ID })
      }
    );
    const data = await response.json();
    if (!response.ok)
      throw new Error(`device authorization failed: ${response.status}`);
    if (!data.device_auth_id || !(data.user_code ?? data.usercode))
      throw new Error("invalid device authorization response");
    const id = crypto.randomUUID(), interval = Math.max(1, Number(data.interval ?? 5)), expires_at = Date.now() + Math.min(900, Number(data.expires_in ?? 900)) * 1e3;
    if (!Number.isFinite(interval) || !Number.isFinite(expires_at))
      throw new Error("invalid device authorization timing");
    const state = {
      id,
      proxy,
      device_auth_id: data.device_auth_id,
      user_code: data.user_code ?? data.usercode,
      interval,
      expires_at,
      next: 0
    };
    const registered = this.registering.then(async () => {
      const ids = await this.ctx.storage.get("logins") ?? [];
      await this.ctx.storage.put("login:" + id, state);
      await this.ctx.storage.put("logins", [...ids, id]);
    });
    this.registering = registered.catch(() => {
    });
    await registered;
    return {
      id,
      user_code: state.user_code,
      verify_url: data.verification_uri ?? authBase(this.env) + "/codex/device",
      expires_at,
      interval
    };
  }
  async poll(id) {
    if (!this.polling.has(id))
      this.polling.set(
        id,
        this.pollOnce(id).finally(() => this.polling.delete(id))
      );
    return this.polling.get(id);
  }
  async pollOnce(id) {
    const key = "login:" + id, state = await this.ctx.storage.get(key);
    if (!state || state.expires_at <= Date.now()) return { state: "expired" };
    if (state.done) return state.done;
    if (state.next > Date.now())
      return {
        state: "slow_down",
        retry_after: Math.ceil((state.next - Date.now()) / 1e3)
      };
    state.next = Date.now() + state.interval * 1e3;
    await this.ctx.storage.put(key, state);
    const response = await this.ctx.fetch(
      authBase(this.env) + "/api/accounts/deviceauth/token",
      {
        method: "POST",
        proxy: state.proxy,
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          device_auth_id: state.device_auth_id,
          user_code: state.user_code
        })
      }
    );
    const data = await response.json();
    if (!response.ok) {
      const code = data.error?.code ?? data.error;
      if (code === "expired_token" || response.status === 410)
        return { state: "expired" };
      if (code === "slow_down" || response.status === 429) {
        state.interval += 5;
        state.next = Date.now() + state.interval * 1e3;
        await this.ctx.storage.put(key, state);
        return { state: "slow_down", retry_after: state.interval };
      }
      if (code === "authorization_pending" || [403, 404].includes(response.status))
        return { state: "pending", retry_after: state.interval };
      throw new Error(`device poll failed: ${code ?? response.status}`);
    }
    if (!data.authorization_code || !data.code_verifier)
      throw new Error("invalid device token response");
    const exchanged = await this.ctx.fetch(
      authBase(this.env) + "/oauth/token",
      {
        method: "POST",
        proxy: state.proxy,
        headers: { "content-type": "application/x-www-form-urlencoded" },
        body: new URLSearchParams({
          grant_type: "authorization_code",
          client_id: CLIENT_ID,
          code: data.authorization_code,
          code_verifier: data.code_verifier,
          redirect_uri: authBase(this.env) + "/deviceauth/callback"
        })
      }
    );
    const tokens = await exchanged.json();
    if (!exchanged.ok)
      throw new Error(`token exchange failed: ${exchanged.status}`);
    const claims = jwt(
      tokens.id_token ?? (tokens.access_token?.split(".").length === 3 ? tokens.access_token : void 0)
    );
    const accountId = claims["https://api.openai.com/auth"]?.chatgpt_account_id ?? claims.chatgpt_account_id;
    if (!accountId) throw new Error("OAuth response lacks account identity");
    const installed = await credential(this.env, accountId).install(
      tokenState(accountId, { proxy: state.proxy }, tokens)
    );
    await this.register(accountId);
    state.done = { state: "done", account: installed };
    await this.ctx.storage.put(key, {
      done: state.done,
      expires_at: state.expires_at
    });
    return state.done;
  }
};

// src/codex.js
var GatewayError = class extends Error {
  constructor(status, message, code = "invalid_request_error") {
    super(message);
    this.status = status;
    this.code = code;
  }
};
function clientKey(request, env) {
  const match = /^Bearer ([^\s]+)$/i.exec(
    request.headers.get("authorization") ?? ""
  );
  const keys = env.CLIENT_API_KEYS;
  if (!Array.isArray(keys) || !keys.length)
    throw new GatewayError(
      401,
      "Client API keys are not configured",
      "invalid_api_key"
    );
  const entry = keys.find(
    (k) => k && typeof k.id === "string" && k.id && typeof k.key === "string" && k.key && k.key !== env.ADMIN_TOKEN && k.enabled !== false && k.key === match?.[1]
  );
  if (!entry) throw new GatewayError(401, "Invalid API key", "invalid_api_key");
  return entry;
}
function prepare(value, env, key, compact = false) {
  if (!value || Array.isArray(value) || typeof value !== "object")
    throw new GatewayError(400, "Expected a JSON object");
  if (typeof value.model !== "string" || !value.model.trim())
    throw new GatewayError(400, "model is required");
  if (key.models !== void 0 && (!Array.isArray(key.models) || !key.models.includes(value.model)))
    throw new GatewayError(
      403,
      "API key cannot access this model",
      "model_not_allowed"
    );
  if (value.previous_response_id || value.conversation)
    throw new GatewayError(
      400,
      "This Codex HTTP adapter requires full input history; previous_response_id/conversation are not supported",
      "unsupported_parameter"
    );
  if (value.background === true)
    throw new GatewayError(
      400,
      "Background responses are not supported",
      "unsupported_parameter"
    );
  if (value.stream !== void 0 && typeof value.stream !== "boolean")
    throw new GatewayError(400, "stream must be a boolean");
  if (compact && value.stream === true)
    throw new GatewayError(400, "compact does not support streaming");
  const body = JSON.parse(JSON.stringify(value));
  if (typeof body.input === "string")
    body.input = [
      {
        type: "message",
        role: "user",
        content: [{ type: "input_text", text: body.input }]
      }
    ];
  if (!Array.isArray(body.input))
    throw new GatewayError(400, "input must be a string or array");
  for (const item of body.input) {
    if (!item || typeof item !== "object" || Array.isArray(item))
      throw new GatewayError(400, "invalid input item");
    if (item.role === "system") item.role = "developer";
    if (Array.isArray(item.content)) {
      for (const part of item.content)
        if (part && typeof part === "object")
          delete part.prompt_cache_breakpoint;
    }
  }
  if (env.MODEL_MAP && Object.hasOwn(env.MODEL_MAP, body.model))
    body.model = env.MODEL_MAP[body.model];
  if (typeof body.model !== "string" || !body.model)
    throw new GatewayError(400, "Invalid model mapping");
  body.instructions ??= "";
  if (typeof body.instructions !== "string")
    throw new GatewayError(400, "instructions must be a string");
  body.store = false;
  if (compact) delete body.stream;
  else {
    body.stream = true;
    if (body.include !== void 0 && !Array.isArray(body.include))
      throw new GatewayError(400, "include must be an array");
    body.include = [
      .../* @__PURE__ */ new Set([...body.include ?? [], "reasoning.encrypted_content"])
    ];
  }
  if (body.tools !== void 0 && !Array.isArray(body.tools))
    throw new GatewayError(400, "tools must be an array");
  const normalizeTool = (tool) => {
    if (tool && ["web_search_preview", "web_search_preview_2025_03_11"].includes(
      tool.type
    ))
      tool.type = "web_search";
  };
  for (const tool of body.tools ?? []) normalizeTool(tool);
  normalizeTool(body.tool_choice);
  for (const tool of body.tool_choice?.tools ?? []) normalizeTool(tool);
  if (body.tools?.length) body.parallel_tool_calls ??= true;
  else delete body.parallel_tool_calls;
  for (const field of [
    "max_tokens",
    "max_output_tokens",
    "max_completion_tokens",
    "temperature",
    "top_p",
    "truncation",
    "prompt_cache_options",
    "prompt_cache_retention",
    "user",
    "context_management",
    "previous_response_id",
    "generate",
    "safety_identifier",
    "stream_options",
    "background"
  ])
    delete body[field];
  if (body.service_tier !== "priority") delete body.service_tier;
  return body;
}
function upstreamHeaders(request, state, env, compact) {
  const headers = new Headers({
    authorization: `Bearer ${state.access}`,
    "chatgpt-account-id": state.accountId,
    "content-type": "application/json",
    accept: compact ? "application/json" : "text/event-stream",
    originator: env.ORIGINATOR ?? "codex_cli_rs",
    "user-agent": env.UPSTREAM_USER_AGENT ?? "aigateway-codex/1.0"
  });
  for (const name of [
    "version",
    "x-codex-beta-features",
    "x-codex-turn-metadata",
    "x-codex-turn-state",
    "x-client-request-id",
    "x-codex-window-id",
    "thread-id",
    "session-id",
    "x-openai-internal-codex-responses-lite"
  ])
    if (request.headers.has(name)) headers.set(name, request.headers.get(name));
  return headers;
}
function responseHeaders(upstream, stream2) {
  const headers = new Headers({
    "content-type": stream2 ? "text/event-stream" : "application/json",
    "cache-control": "no-store"
  });
  for (const name of ["x-request-id", "x-codex-turn-state", "retry-after"])
    if (upstream.headers.has(name))
      headers.set(name, upstream.headers.get(name));
  return headers;
}
function retrySeconds(response) {
  const value = response.headers.get("retry-after");
  const seconds = value && /^\d+$/.test(value) ? Number(value) : value ? (Date.parse(value) - Date.now()) / 1e3 : 60;
  return Number.isFinite(seconds) ? Math.min(3600, Math.max(1, Math.ceil(seconds))) : 60;
}

// src/stream.js
var encoder = new TextEncoder();
var MAX_FRAME = 1 << 20;
var MAX_RESULT = 8 << 20;
async function* frames(body, cancellation = {}) {
  if (!body)
    throw new GatewayError(
      502,
      "Upstream response has no body",
      "upstream_error"
    );
  const reader = body.getReader(), decoder = new TextDecoder();
  cancellation.cancel = () => reader.cancel();
  let pending = "";
  const parse = (raw2) => {
    const data = raw2.replace(/\r\n/g, "\n").split("\n").filter((l) => l.startsWith("data:")).map((l) => l.slice(5).replace(/^ /, "")).join("\n");
    if (!data || data === "[DONE]") return { raw: raw2, value: null };
    return { raw: raw2, value: JSON.parse(data) };
  };
  try {
    for (; ; ) {
      const { value, done } = await reader.read();
      pending += decoder.decode(value, { stream: !done });
      for (; ; ) {
        const end = /\r?\n\r?\n/.exec(pending);
        if (!end) break;
        const raw2 = pending.slice(0, end.index + end[0].length);
        pending = pending.slice(raw2.length);
        if (encoder.encode(raw2).length > MAX_FRAME)
          throw new Error("SSE frame exceeds 1 MiB");
        yield parse(raw2);
      }
      if (encoder.encode(pending).length > MAX_FRAME)
        throw new Error("SSE frame exceeds 1 MiB");
      if (done) {
        if (pending.trim()) yield parse(pending + "\n\n");
        return;
      }
    }
  } finally {
    await reader.cancel().catch(() => {
    });
    reader.releaseLock();
  }
}
function terminal(value) {
  return [
    "response.completed",
    "response.incomplete",
    "response.failed",
    "response.cancelled",
    "error"
  ].includes(value?.type);
}
function failure(value) {
  if (["error", "response.failed", "response.cancelled"].includes(value?.type) || ["failed", "cancelled", "canceled"].includes(value?.response?.status) || value?.response?.error)
    return value.error?.message ?? value.response?.error?.message ?? "Codex generation failed";
}
async function aggregate(body, onResult) {
  const items = /* @__PURE__ */ new Map();
  let size = 0;
  for await (const { value } of frames(body)) {
    if (!value) continue;
    const error = failure(value);
    if (error) throw new GatewayError(502, error, "upstream_error");
    if (value.type === "response.output_item.done" && value.item) {
      const index = value.output_index ?? items.size;
      size += encoder.encode(JSON.stringify(value.item)).length - (items.has(index) ? encoder.encode(JSON.stringify(items.get(index))).length : 0);
      if (size > MAX_RESULT)
        throw new GatewayError(
          502,
          "Aggregated response exceeds 8 MiB",
          "upstream_error"
        );
      items.set(index, value.item);
    }
    if (terminal(value)) {
      const result = value.response;
      if (!result || !["completed", "incomplete"].includes(result.status))
        throw new GatewayError(
          502,
          "Invalid Codex terminal response",
          "upstream_error"
        );
      if (!result.output?.length && items.size)
        result.output = [...items].sort(([a], [b]) => a - b).map(([, item]) => item);
      await onResult(result);
      return result;
    }
  }
  throw new GatewayError(
    502,
    "Upstream ended before a Responses terminal event",
    "upstream_error"
  );
}
function stream(body, onResult) {
  const cancellation = {};
  const source = frames(body, cancellation);
  let ended = false;
  return new ReadableStream({
    async pull(controller) {
      try {
        if (ended) {
          controller.close();
          return;
        }
        const next = await source.next();
        if (next.done)
          throw new Error("Upstream ended before a Responses terminal event");
        const { raw: raw2, value } = next.value;
        if (terminal(value)) {
          if (value.response) await onResult(value.response);
          ended = true;
          await source.return();
        }
        controller.enqueue(encoder.encode(raw2));
      } catch (error) {
        ended = true;
        await source.return().catch(() => {
        });
        controller.enqueue(
          encoder.encode(
            `event: error
data: ${JSON.stringify({ type: "error", code: "upstream_error", message: error.message })}

`
          )
        );
        controller.close();
      }
    },
    async cancel() {
      ended = true;
      await cancellation.cancel?.();
      await source.return();
    }
  });
}

// src/index.js
var app = new Hono2();
var fail = (c, status, message, code) => c.json(
  {
    error: { message, type: code ?? "api_error", code: code ?? "api_error" }
  },
  status
);
app.use("/codex/admin/*", async (c, next) => {
  if (!c.env.ADMIN_TOKEN || c.req.header("authorization") !== `Bearer ${c.env.ADMIN_TOKEN}`)
    return fail(c, 401, "Invalid administrator token", "invalid_api_key");
  await next();
});
app.get("/codex/admin/accounts", async (c) => {
  const ids = await c.env.CONFIG.get("accounts", { type: "json" }) ?? [];
  return c.json(
    await Promise.all(ids.map((id) => credential(c.env, id).status()))
  );
});
app.post("/codex/admin/accounts/import", async (c) => {
  const value = await c.req.json();
  const tokens = value.tokens ?? value;
  const access = tokens.access_token;
  const claims = jwt(
    tokens.id_token ?? (typeof access === "string" && access.split(".").length === 3 ? access : void 0)
  );
  const id = tokens.account_id ?? value.account_id ?? claims["https://api.openai.com/auth"]?.chatgpt_account_id;
  if (typeof access !== "string" || !access || typeof id !== "string" || !id || id.length > 256)
    throw new GatewayError(
      400,
      "access_token and account_id (or token claims) are required"
    );
  const expiresAt = value.expires_at !== void 0 ? Number(value.expires_at) : value.expired ? Date.parse(value.expired) : claims.exp ? claims.exp * 1e3 : Date.now() + 36e5;
  if (!Number.isFinite(expiresAt) || expiresAt < 0)
    throw new GatewayError(400, "Invalid expiry");
  if (tokens.refresh_token !== void 0 && typeof tokens.refresh_token !== "string")
    throw new GatewayError(400, "Invalid refresh token");
  const proxy = value.proxy ?? "direct";
  validateProxy(proxy);
  const result = await credential(c.env, id).importState({
    id,
    accountId: id,
    access,
    refresh: tokens.refresh_token,
    expiresAt,
    proxy,
    state: "active",
    enabled: true,
    cooldown: 0
  });
  await directory(c.env).register(id);
  return c.json(result, 201);
});
app.put("/codex/admin/accounts/:id/enabled", async (c) => {
  const value = await c.req.json();
  if (typeof value.enabled !== "boolean")
    throw new GatewayError(400, "enabled must be a boolean");
  return c.json(
    await credential(c.env, c.req.param("id")).setEnabled(value.enabled)
  );
});
function validateProxy(proxy) {
  if (typeof proxy !== "string" || proxy !== "direct" && !/^(https?|socks5h?):\/\//.test(proxy))
    throw new GatewayError(400, "Unsupported proxy URL");
}
app.post("/codex/admin/accounts/login", async (c) => {
  const proxy = c.req.query("proxy") ?? "direct";
  validateProxy(proxy);
  return c.json(await directory(c.env).login(proxy));
});
app.post("/codex/admin/accounts/login/:id/poll", async (c) => {
  const result = await directory(c.env).poll(c.req.param("id"));
  return c.json(result, result.state === "expired" ? 410 : 200);
});
app.get(
  "/codex/admin/usage",
  async (c) => c.json(
    await c.env.USAGE.query({
      since: Number(c.req.query("since") ?? 0),
      groupBy: ["credential", "model"]
    })
  )
);
for (const path of ["/v1/models", "/codex/models"])
  app.get(path, (c) => {
    const key = clientKey(c.req.raw, c.env);
    const models = c.env.MODELS ?? Object.keys(c.env.MODEL_MAP ?? {});
    return c.json({
      object: "list",
      data: models.filter(
        (m) => typeof m === "string" && (!key.models || key.models.includes(m))
      ).map((id) => ({ id, object: "model", owned_by: "codex" }))
    });
  });
async function digest(text) {
  const bytes = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(text)
  );
  return [...new Uint8Array(bytes)].map((v) => v.toString(16).padStart(2, "0")).join("");
}
async function record(env, value, account2, caller) {
  if (!value?.id || !value.usage || !await directory(env).claimUsage(account2 + ":" + value.id))
    return;
  const u = value.usage;
  env.USAGE.record(
    Object.fromEntries(
      Object.entries({
        credential: account2,
        caller,
        model: value.model ?? "",
        input: u.input_tokens,
        output: u.output_tokens,
        cached: u.input_tokens_details?.cached_tokens,
        reasoning: u.output_tokens_details?.reasoning_tokens
      }).filter(([, v]) => v !== void 0)
    )
  );
}
async function respond(c) {
  const request = c.req.raw, env = c.env;
  const key = clientKey(request, env);
  const input = await c.req.json();
  const compact = new URL(request.url).pathname.endsWith("/compact");
  const body = prepare(input, env, key, compact);
  const wantStream = !compact && input.stream === true;
  const session = body.prompt_cache_key ?? request.headers.get("session-id") ?? request.headers.get("thread-id");
  if (session !== void 0 && session !== null && typeof session !== "string")
    throw new GatewayError(400, "session/cache key must be a string");
  const sticky = session ? "session:" + await digest(key.id + "\0" + session) : null;
  const preferred = sticky ? await env.CONFIG.get(sticky) : null;
  const candidates = await directory(env).candidates(preferred);
  const base = (env.UPSTREAM ?? "https://chatgpt.com/backend-api/codex").replace(/\/$/, "");
  let lastError;
  for (const id of candidates) {
    if (request.signal.aborted) throw request.signal.reason;
    let state;
    try {
      state = await credential(env, id).token();
    } catch {
      continue;
    }
    let response;
    for (let attempt = 0; attempt < 2; attempt++) {
      response = await c.executionCtx.fetch(
        base + (compact ? "/responses/compact" : "/responses"),
        {
          method: "POST",
          proxy: state.proxy ?? "direct",
          headers: upstreamHeaders(request, state, env, compact),
          body: JSON.stringify(body),
          signal: request.signal
        }
      );
      if (response.status !== 401 || attempt === 1 || !state.refresh) break;
      await response.body?.cancel();
      response = null;
      try {
        state = await credential(env, id).token(true, state.access);
      } catch {
        break;
      }
    }
    if (!response) continue;
    if ([401, 429].includes(response.status)) {
      const seconds = response.status === 429 ? retrySeconds(response) : 30;
      await response.body?.cancel();
      await credential(env, id).cooldown(seconds);
      lastError = { status: response.status, seconds };
      continue;
    }
    if (!response.ok) {
      const headers2 = responseHeaders(response, false);
      let data;
      try {
        data = await response.json();
      } catch {
        data = {};
      }
      return Response.json(
        {
          error: {
            message: data.error?.message ?? data.detail ?? `Codex upstream returned ${response.status}`,
            type: data.error?.type ?? "upstream_error",
            code: data.error?.code ?? "upstream_error"
          }
        },
        { status: response.status, headers: headers2 }
      );
    }
    if (sticky) await env.CONFIG.put(sticky, id, { expirationTtl: 3600 });
    const headers = responseHeaders(response, wantStream);
    if (compact) {
      const value = await response.json();
      if (value.error)
        return fail(
          c,
          502,
          value.error.message ?? "Compaction failed",
          "upstream_error"
        );
      await record(env, value, id, key.id);
      return Response.json(value, { headers });
    }
    if (!response.headers.get("content-type")?.includes("text/event-stream")) {
      await response.body?.cancel();
      return fail(
        c,
        502,
        "Codex upstream did not return SSE",
        "upstream_error"
      );
    }
    const onResult = (value) => record(env, value, id, key.id);
    if (wantStream)
      return new Response(stream(response.body, onResult), { headers });
    return Response.json(await aggregate(response.body, onResult), { headers });
  }
  if (lastError?.status === 429)
    c.header("retry-after", String(lastError.seconds));
  return fail(
    c,
    lastError?.status === 429 ? 429 : 503,
    "No available Codex account",
    lastError?.status === 429 ? "rate_limit_error" : "account_unavailable"
  );
}
for (const path of [
  "/v1/responses",
  "/v1/responses/compact",
  "/codex/responses",
  "/codex/responses/compact"
])
  app.post(path, respond);
app.notFound((c) => fail(c, 404, "Unknown Codex route", "not_found"));
app.onError(
  (error, c) => fail(
    c,
    error.status ?? (error instanceof SyntaxError ? 400 : 502),
    error.message,
    error.code
  )
);
var index_default = {
  fetch: (r, env, ctx) => app.fetch(r, env, ctx),
  async scheduled(event, env, ctx) {
    const ids = await env.CONFIG.get("accounts", { type: "json" }) ?? [];
    ctx.waitUntil(
      Promise.allSettled([
        ...ids.map((id) => credential(env, id).maintain()),
        directory(env).pruneLogins()
      ])
    );
  }
};
export {
  Credential,
  Pool,
  index_default as default
};
