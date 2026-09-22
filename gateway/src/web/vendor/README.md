# Vendored Streams implementation

- Package: `web-streams-polyfill@4.2.0`, `dist/polyfill.js`.
- Upstream: https://github.com/MattiasBuelens/web-streams-polyfill
- Source tarball: https://registry.npmjs.org/web-streams-polyfill/-/web-streams-polyfill-4.2.0.tgz
- License: MIT, preserved in `streams.LICENSE`.
- SHA-256 of the vendored file: `14e79306fcdbee45c42eb4b53dee2a448aa9f8b7893cd42b6d16791cc8f025e0`.
- npm package integrity: `sha512-0rYDzGOh9EZpig92umN5g5D/9A1Kff7k0/mzPSSCY8jEQeYkgRMoY7LhbXtUCWzLCMX0TUE9aoHkjFNB7D9pfA==`.

The vendored file is unmodified. `../streams.js` adds UTF-8 transform
streams and excludes BYOB/byte sources from the supported public surface.
`../http.js` uses this pinned implementation's `_disturbed` flag to enforce
BodyInit/bodyUsed checks; an upgrade must rerun the body and stream suites.
There is no Node runtime dependency.
