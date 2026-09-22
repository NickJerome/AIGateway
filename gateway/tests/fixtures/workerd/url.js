// Ported assertions from workerd's src/workerd/api/tests/url-test.js
// (cloudflare/workerd, Apache-2.0). Only pure parse/serialize semantics with
// no network and no platform dependence are ported; see README.md in this
// directory for the full port/exclusion table and registered deviations.
// Every section carries a `// workerd: <file>:<start>-<end>` provenance mark.
//
// The host harness prepends node-assert-shim.mjs, which provides
// strictEqual / deepStrictEqual / ok / throws / match / fail.

export default {
  async fetch() {
    // workerd: url-test.js:6-141 (constructAndGet)
    {
      const cases = [
        { input: 'https://capnproto.org', pathname: '/' },
        { input: 'https://capnproto.org/', pathname: '/' },
        { input: 'https://capnproto.org/foo', pathname: '/foo' },
        { input: 'https://capnproto.org/foo/', pathname: '/foo/' },
        { input: 'https://capnproto.org/%2F', pathname: '/%2F' },
        { input: 'https://capnproto.org//', pathname: '//' },
        { input: 'https://capnproto.org/./', pathname: '/' },
        { input: 'https://capnproto.org/?<>', search: '?%3C%3E' },
        { input: 'https://capnproto.org/??', search: '??' },
        { input: 'https://capnproto.org/?foo', search: '?foo', href: 'https://capnproto.org/?foo' },
        { input: 'https://capnproto.org/?foo=', search: '?foo=', href: 'https://capnproto.org/?foo=' },
        { input: 'https://capnproto.org/?foo=&bar', search: '?foo=&bar', href: 'https://capnproto.org/?foo=&bar' },
        { input: 'https://capnproto.org/?foo&bar=', search: '?foo&bar=', href: 'https://capnproto.org/?foo&bar=' },
        { input: 'https://capnproto.org/?foo+bar=baz+qux', search: '?foo+bar=baz+qux' },
        { input: 'https://capnproto.org/?\u{1F63A}', search: '?%F0%9F%98%BA' },
        { input: 'https://capnproto.org/#\u{1F63A}', hash: '#%F0%9F%98%BA' },
        { input: 'https://capnproto.org:443/', href: 'https://capnproto.org/' },
        { input: 'https://capnproto.org:/', href: 'https://capnproto.org/' },
        { input: 'http://foo/bar;baz@qux', href: 'http://foo/bar;baz@qux' },
        { input: 'https://jam_com.helpusersvote.net/', href: 'https://jam_com.helpusersvote.net/' },
        {
          input: 'https://foo%25bar:baz%25qux@capnproto.org/foo%25bar?foo%25bar=baz%25qux#foo%25bar',
          href: 'https://foo%25bar:baz%25qux@capnproto.org/foo%25bar?foo%25bar=baz%25qux#foo%25bar',
        },
      ];
      // workerd: url-test.js:128-139 (constructAndGet assertion loop)
      cases.forEach((test) => {
        ['pathname', 'search', 'hash', 'href'].forEach((attribute) => {
          const url = new URL(test.input);
          if (test[attribute] !== undefined) {
            strictEqual(url[attribute], test[attribute], `${test.input}`);
          }
        });
      });
    }

    // workerd: url-test.js:143-293 (constructSetAndGet)
    {
      const cases = {
        protocol: [
          {
            href: 'https://example.com/',
            new_value: 'http',
            expected: { href: 'http://example.com/' },
          },
          {
            href: 'https://example.com:80/',
            new_value: 'http',
            expected: { href: 'http://example.com/' },
          },
          {
            href: 'https://example.com:1234/',
            new_value: 'http',
            expected: { href: 'http://example.com:1234/' },
          },
        ],
        search: [
          {
            href: 'https://capnproto.org/',
            new_value: '#',
            expected: { href: 'https://capnproto.org/?%23', search: '?%23' },
          },
          {
            href: 'https://capnproto.org/',
            new_value: '#=#',
            expected: { href: 'https://capnproto.org/?%23=%23', search: '?%23=%23' },
          },
        ],
        host: [
          {
            href: 'https://example.com/',
            new_value: 'foo.example.com:',
            expected: { href: 'https://foo.example.com/' },
          },
          {
            href: 'https://example.com/',
            new_value: 'foo.example.com:80',
            expected: { href: 'https://foo.example.com:80/' },
          },
          {
            href: 'https://example.com:1234/',
            new_value: 'foo.example.com:443',
            expected: { href: 'https://foo.example.com/' },
          },
          {
            href: 'https://example.com/',
            new_value: 'foo.example.com:443',
            expected: { href: 'https://foo.example.com/' },
          },
        ],
        port: [
          { href: 'https://example.com/', new_value: '443', expected: { href: 'https://example.com/' } },
          { href: 'https://example.com:1234/', new_value: '443', expected: { href: 'https://example.com/' } },
          { href: 'https://example.com/', new_value: '1234', expected: { href: 'https://example.com:1234/' } },
          { href: 'https://example.com:80/', new_value: '1234', expected: { href: 'https://example.com:1234/' } },
          { href: 'https://example.com:1234/', new_value: '', expected: { href: 'https://example.com/' } },
        ],
        hostname: [
          {
            href: 'http://example.com/path/to/something?query=foo#hash',
            new_value: 'newexample.com',
            expected: { href: 'http://newexample.com/path/to/something?query=foo#hash' },
          },
        ],
      };
      // workerd: url-test.js:279-291 (constructSetAndGet assertion loop)
      for (const attribute in cases) {
        cases[attribute].forEach((test) => {
          const url = new URL(test.href);
          url[attribute] = test.new_value;
          for (const a in test.expected) {
            strictEqual(url[a], test.expected[a], `${test.href}`);
          }
        });
      }
    }

    // workerd: url-test.js:295-302 (urlSearchParamsStringifyBehavior)
    {
      const url = new URL('https://example.com/?foo&bar=&baz=qux');
      strictEqual(url.href, 'https://example.com/?foo&bar=&baz=qux');
      strictEqual(url.search, '?foo&bar=&baz=qux');
      strictEqual(url.searchParams.toString(), 'foo=&bar=&baz=qux');
    }

    // workerd: url-test.js:304-350 (urlSearchParamsForEach)
    {
      let searchParams = new URLSearchParams('');
      searchParams.forEach(() => {
        fail('Should not have been called');
      });

      const foreachOutput = [];
      searchParams = new URLSearchParams('key1=value1&key2=value2');
      strictEqual(searchParams.size, 2);
      let pushed = false;
      searchParams.forEach((val, key) => {
        foreachOutput.push([key, val]);
        if (!pushed) {
          // We can add params within this loop but it's not really safe
          // because it will cause the loop to run forever if we're not
          // careful.
          pushed = true;
          searchParams.set('key3', 'value3');
        }
      });
      deepStrictEqual(foreachOutput, [
        ['key1', 'value1'],
        ['key2', 'value2'],
        ['key3', 'value3'],
      ]);
      strictEqual(searchParams.size, 3);

      // Calling forEach with no argument throws
      throws(() => searchParams.forEach());

      // Calling forEach with wrong arguments throws
      throws(() => searchParams.forEach(1));

      // This can be overridden by the second argument
      searchParams.forEach(function () {
        strictEqual(this, 1);
      }, 1);

      // Propagates errors
      throws(() =>
        searchParams.forEach(() => {
          throw new Error('boom');
        })
      );
    }

    // workerd: url-test.js:352-358 (urlSearchParamsInit1)
    {
      const search1 = new URLSearchParams('a=b');
      const search2 = new URLSearchParams(search1);
      strictEqual(search1.toString(), search2.toString());
    }

    // workerd: url-test.js:409-414 (urlToJson)
    {
      const url = new URL('https://example.com');
      strictEqual(JSON.stringify(url), '"https://example.com/"');
    }

    // workerd: url-test.js:416-430 (urlSearchParamsSet)
    {
      const url = new URL('https://example.com?c=d');
      url.searchParams.set('a', 'b');
      strictEqual(url.searchParams.size, 2);
      url.searchParams.delete('c');
      strictEqual(url.searchParams.size, 1);
      ok(url.searchParams.has('a'));
      ok(!url.searchParams.has('c'));
      ok(url.searchParams.has('a', 'b'));
      ok(!url.searchParams.has('a', 'c'));
      url.searchParams.delete('a', 'c');
      ok(url.searchParams.has('a'));
    }

    // workerd: url-test.js:432-6896 (urlConstructorTests): ported subset of
    // the WPT/WebKit-derived case table plus its verbatim assertion loop.
    {
      const cases = [
    // workerd: url-test.js:436-450 (urlConstructorTests case data)
      {
        input: 'http://example\t.\norg',
        base: 'http://example.org/foo/bar',
        href: 'http://example.org/',
        origin: 'http://example.org',
        protocol: 'http:',
        username: '',
        password: '',
        host: 'example.org',
        hostname: 'example.org',
        port: '',
        pathname: '/',
        search: '',
        hash: '',
      },
    // workerd: url-test.js:479-717 (urlConstructorTests case data)
    // Excluded: url-test.js:451-478 (C|/ drive-letter cases) depend on the
    // current WHATWG file: host retention rule; rust-url follows the older
    // rule and drops the host. Registered in README.md.
      {
        input: 'https://:@test',
        base: 'about:blank',
        href: 'https://test/',
        origin: 'https://test',
        protocol: 'https:',
        username: '',
        password: '',
        host: 'test',
        hostname: 'test',
        port: '',
        pathname: '/',
        search: '',
        hash: '',
      },
      {
        input: 'non-special://test:@test/x',
        base: 'about:blank',
        href: 'non-special://test@test/x',
        origin: 'null',
        protocol: 'non-special:',
        username: 'test',
        password: '',
        host: 'test',
        hostname: 'test',
        port: '',
        pathname: '/x',
        search: '',
        hash: '',
      },
      {
        input: 'non-special://:@test/x',
        base: 'about:blank',
        href: 'non-special://test/x',
        origin: 'null',
        protocol: 'non-special:',
        username: '',
        password: '',
        host: 'test',
        hostname: 'test',
        port: '',
        pathname: '/x',
        search: '',
        hash: '',
      },
      {
        input: 'http:foo.com',
        base: 'http://example.org/foo/bar',
        href: 'http://example.org/foo/foo.com',
        origin: 'http://example.org',
        protocol: 'http:',
        username: '',
        password: '',
        host: 'example.org',
        hostname: 'example.org',
        port: '',
        pathname: '/foo/foo.com',
        search: '',
        hash: '',
      },
      {
        input: '\t   :foo.com   \n',
        base: 'http://example.org/foo/bar',
        href: 'http://example.org/foo/:foo.com',
        origin: 'http://example.org',
        protocol: 'http:',
        username: '',
        password: '',
        host: 'example.org',
        hostname: 'example.org',
        port: '',
        pathname: '/foo/:foo.com',
        search: '',
        hash: '',
      },
      {
        input: ' foo.com  ',
        base: 'http://example.org/foo/bar',
        href: 'http://example.org/foo/foo.com',
        origin: 'http://example.org',
        protocol: 'http:',
        username: '',
        password: '',
        host: 'example.org',
        hostname: 'example.org',
        port: '',
        pathname: '/foo/foo.com',
        search: '',
        hash: '',
      },
      {
        input: 'a:\t foo.com',
        base: 'http://example.org/foo/bar',
        href: 'a: foo.com',
        origin: 'null',
        protocol: 'a:',
        username: '',
        password: '',
        host: '',
        hostname: '',
        port: '',
        pathname: ' foo.com',
        search: '',
        hash: '',
      },
      {
        input: 'http://f:21/ b ? d # e ',
        base: 'http://example.org/foo/bar',
        href: 'http://f:21/%20b%20?%20d%20#%20e',
        origin: 'http://f:21',
        protocol: 'http:',
        username: '',
        password: '',
        host: 'f:21',
        hostname: 'f',
        port: '21',
        pathname: '/%20b%20',
        search: '?%20d%20',
        hash: '#%20e',
      },
      {
        input: 'lolscheme:x x#x x',
        base: 'about:blank',
        href: 'lolscheme:x x#x%20x',
        protocol: 'lolscheme:',
        username: '',
        password: '',
        host: '',
        hostname: '',
        port: '',
        pathname: 'x x',
        search: '',
        hash: '#x%20x',
      },
      {
        input: 'http://f:/c',
        base: 'http://example.org/foo/bar',
        href: 'http://f/c',
        origin: 'http://f',
        protocol: 'http:',
        username: '',
        password: '',
        host: 'f',
        hostname: 'f',
        port: '',
        pathname: '/c',
        search: '',
        hash: '',
      },
      {
        input: 'http://f:0/c',
        base: 'http://example.org/foo/bar',
        href: 'http://f:0/c',
        origin: 'http://f:0',
        protocol: 'http:',
        username: '',
        password: '',
        host: 'f:0',
        hostname: 'f',
        port: '0',
        pathname: '/c',
        search: '',
        hash: '',
      },
      {
        input: 'http://f:00000000000000/c',
        base: 'http://example.org/foo/bar',
        href: 'http://f:0/c',
        origin: 'http://f:0',
        protocol: 'http:',
        username: '',
        password: '',
        host: 'f:0',
        hostname: 'f',
        port: '0',
        pathname: '/c',
        search: '',
        hash: '',
      },
      {
        input: 'http://f:00000000000000000000080/c',
        base: 'http://example.org/foo/bar',
        href: 'http://f/c',
        origin: 'http://f',
        protocol: 'http:',
        username: '',
        password: '',
        host: 'f',
        hostname: 'f',
        port: '',
        pathname: '/c',
        search: '',
        hash: '',
      },
      {
        input: 'http://f:b/c',
        base: 'http://example.org/foo/bar',
        failure: true,
      },
      {
        input: 'http://f: /c',
        base: 'http://example.org/foo/bar',
        failure: true,
      },
      {
        input: 'http://f:\n/c',
        base: 'http://example.org/foo/bar',
        href: 'http://f/c',
        origin: 'http://f',
        protocol: 'http:',
        username: '',
        password: '',
        host: 'f',
        hostname: 'f',
        port: '',
        pathname: '/c',
        search: '',
        hash: '',
      },
      {
        input: 'http://f:fifty-two/c',
        base: 'http://example.org/foo/bar',
        failure: true,
      },
      {
        input: 'http://f:999999/c',
        base: 'http://example.org/foo/bar',
        failure: true,
      },
      {
        input: 'non-special://f:999999/c',
        base: 'http://example.org/foo/bar',
        failure: true,
      },
      {
        input: 'http://f: 21 / b ? d # e ',
        base: 'http://example.org/foo/bar',
        failure: true,
      },
    // workerd: url-test.js:1018-1137 (urlConstructorTests case data)
      {
        input: 'http://a:b@c:29/d',
        base: 'http://example.org/foo/bar',
        href: 'http://a:b@c:29/d',
        origin: 'http://c:29',
        protocol: 'http:',
        username: 'a',
        password: 'b',
        host: 'c:29',
        hostname: 'c',
        port: '29',
        pathname: '/d',
        search: '',
        hash: '',
      },
      {
        input: 'http::@c:29',
        base: 'http://example.org/foo/bar',
        href: 'http://example.org/foo/:@c:29',
        origin: 'http://example.org',
        protocol: 'http:',
        username: '',
        password: '',
        host: 'example.org',
        hostname: 'example.org',
        port: '',
        pathname: '/foo/:@c:29',
        search: '',
        hash: '',
      },
      {
        input: 'http://&a:foo(b]c@d:2/',
        base: 'http://example.org/foo/bar',
        href: 'http://&a:foo(b%5Dc@d:2/',
        origin: 'http://d:2',
        protocol: 'http:',
        username: '&a',
        password: 'foo(b%5Dc',
        host: 'd:2',
        hostname: 'd',
        port: '2',
        pathname: '/',
        search: '',
        hash: '',
      },
      {
        input: 'http://::@c@d:2',
        base: 'http://example.org/foo/bar',
        href: 'http://:%3A%40c@d:2/',
        origin: 'http://d:2',
        protocol: 'http:',
        username: '',
        password: '%3A%40c',
        host: 'd:2',
        hostname: 'd',
        port: '2',
        pathname: '/',
        search: '',
        hash: '',
      },
      {
        input: 'http://foo.com:b@d/',
        base: 'http://example.org/foo/bar',
        href: 'http://foo.com:b@d/',
        origin: 'http://d',
        protocol: 'http:',
        username: 'foo.com',
        password: 'b',
        host: 'd',
        hostname: 'd',
        port: '',
        pathname: '/',
        search: '',
        hash: '',
      },
      {
        input: 'http://foo.com/\\@',
        base: 'http://example.org/foo/bar',
        href: 'http://foo.com//@',
        origin: 'http://foo.com',
        protocol: 'http:',
        username: '',
        password: '',
        host: 'foo.com',
        hostname: 'foo.com',
        port: '',
        pathname: '//@',
        search: '',
        hash: '',
      },
      {
        input: 'http:\\\\foo.com\\',
        base: 'http://example.org/foo/bar',
        href: 'http://foo.com/',
        origin: 'http://foo.com',
        protocol: 'http:',
        username: '',
        password: '',
        host: 'foo.com',
        hostname: 'foo.com',
        port: '',
        pathname: '/',
        search: '',
        hash: '',
      },
      {
        input: 'http:\\\\a\\b:c\\d@foo.com\\',
        base: 'http://example.org/foo/bar',
        href: 'http://a/b:c/d@foo.com/',
        origin: 'http://a',
        protocol: 'http:',
        username: '',
        password: '',
        host: 'a',
        hostname: 'a',
        port: '',
        pathname: '/b:c/d@foo.com/',
        search: '',
        hash: '',
      },
    // workerd: url-test.js:1243-1287 (urlConstructorTests case data)
      {
        input: 'http://foo/path;a??e#f#g',
        base: 'http://example.org/foo/bar',
        href: 'http://foo/path;a??e#f#g',
        origin: 'http://foo',
        protocol: 'http:',
        username: '',
        password: '',
        host: 'foo',
        hostname: 'foo',
        port: '',
        pathname: '/path;a',
        search: '??e',
        hash: '#f#g',
      },
      {
        input: 'http://foo/abcd?efgh?ijkl',
        base: 'http://example.org/foo/bar',
        href: 'http://foo/abcd?efgh?ijkl',
        origin: 'http://foo',
        protocol: 'http:',
        username: '',
        password: '',
        host: 'foo',
        hostname: 'foo',
        port: '',
        pathname: '/abcd',
        search: '?efgh?ijkl',
        hash: '',
      },
      {
        input: 'http://foo/abcd#foo?bar',
        base: 'http://example.org/foo/bar',
        href: 'http://foo/abcd#foo?bar',
        origin: 'http://foo',
        protocol: 'http:',
        username: '',
        password: '',
        host: 'foo',
        hostname: 'foo',
        port: '',
        pathname: '/abcd',
        search: '',
        hash: '#foo?bar',
      },
    // workerd: url-test.js:1318-1397 (urlConstructorTests case data)
      {
        input: 'http://[1::2]:3:4',
        base: 'http://example.org/foo/bar',
        failure: true,
      },
      {
        input: 'http://2001::1',
        base: 'http://example.org/foo/bar',
        failure: true,
      },
      {
        input: 'http://2001::1]',
        base: 'http://example.org/foo/bar',
        failure: true,
      },
      {
        input: 'http://2001::1]:80',
        base: 'http://example.org/foo/bar',
        failure: true,
      },
      {
        input: 'http://[2001::1]',
        base: 'http://example.org/foo/bar',
        href: 'http://[2001::1]/',
        origin: 'http://[2001::1]',
        protocol: 'http:',
        username: '',
        password: '',
        host: '[2001::1]',
        hostname: '[2001::1]',
        port: '',
        pathname: '/',
        search: '',
        hash: '',
      },
      {
        input: 'http://[::127.0.0.1]',
        base: 'http://example.org/foo/bar',
        href: 'http://[::7f00:1]/',
        origin: 'http://[::7f00:1]',
        protocol: 'http:',
        username: '',
        password: '',
        host: '[::7f00:1]',
        hostname: '[::7f00:1]',
        port: '',
        pathname: '/',
        search: '',
        hash: '',
      },
      {
        input: 'http://[0:0:0:0:0:0:13.1.68.3]',
        base: 'http://example.org/foo/bar',
        href: 'http://[::d01:4403]/',
        origin: 'http://[::d01:4403]',
        protocol: 'http:',
        username: '',
        password: '',
        host: '[::d01:4403]',
        hostname: '[::d01:4403]',
        port: '',
        pathname: '/',
        search: '',
        hash: '',
      },
      {
        input: 'http://[2001::1]:80',
        base: 'http://example.org/foo/bar',
        href: 'http://[2001::1]/',
        origin: 'http://[2001::1]',
        protocol: 'http:',
        username: '',
        password: '',
        host: '[2001::1]',
        hostname: '[2001::1]',
        port: '',
        pathname: '/',
        search: '',
        hash: '',
      },
    // workerd: url-test.js:1398-1491 (urlConstructorTests case data)
      {
        input: 'http:/example.com/',
        base: 'http://example.org/foo/bar',
        href: 'http://example.org/example.com/',
        origin: 'http://example.org',
        protocol: 'http:',
        username: '',
        password: '',
        host: 'example.org',
        hostname: 'example.org',
        port: '',
        pathname: '/example.com/',
        search: '',
        hash: '',
      },
      {
        input: 'ftp:/example.com/',
        base: 'http://example.org/foo/bar',
        href: 'ftp://example.com/',
        origin: 'ftp://example.com',
        protocol: 'ftp:',
        username: '',
        password: '',
        host: 'example.com',
        hostname: 'example.com',
        port: '',
        pathname: '/',
        search: '',
        hash: '',
      },
      {
        input: 'https:/example.com/',
        base: 'http://example.org/foo/bar',
        href: 'https://example.com/',
        origin: 'https://example.com',
        protocol: 'https:',
        username: '',
        password: '',
        host: 'example.com',
        hostname: 'example.com',
        port: '',
        pathname: '/',
        search: '',
        hash: '',
      },
      {
        input: 'madeupscheme:/example.com/',
        base: 'http://example.org/foo/bar',
        href: 'madeupscheme:/example.com/',
        origin: 'null',
        protocol: 'madeupscheme:',
        username: '',
        password: '',
        host: '',
        hostname: '',
        port: '',
        pathname: '/example.com/',
        search: '',
        hash: '',
      },
      {
        input: 'file:/example.com/',
        base: 'http://example.org/foo/bar',
        href: 'file:///example.com/',
        protocol: 'file:',
        username: '',
        password: '',
        host: '',
        hostname: '',
        port: '',
        pathname: '/example.com/',
        search: '',
        hash: '',
      },
      {
        input: 'file://example:1/',
        base: 'about:blank',
        failure: true,
      },
      {
        input: 'file://example:test/',
        base: 'about:blank',
        failure: true,
      },
      {
        input: 'file://example%/',
        base: 'about:blank',
        failure: true,
      },
      {
        input: 'file://[example]/',
        base: 'about:blank',
        failure: true,
      },
    // workerd: url-test.js:6850-6864 (urlConstructorTests case data)
      {
        input: 'http://example.org/test?a#b\u0000c',
        base: 'about:blank',
        href: 'http://example.org/test?a#b%00c',
        origin: 'http://example.org',
        protocol: 'http:',
        username: '',
        password: '',
        host: 'example.org',
        hostname: 'example.org',
        port: '',
        pathname: '/test',
        search: '?a',
        hash: '#b%00c',
      },
      ];

      // workerd: url-test.js:6873-6896 (urlConstructorTests assertion loop)
      cases.forEach((test) => {
        if (typeof test === 'string') return;
        if (test.failure) {
          throws(() => {
            new URL(test.input, test.base || 'about:blank');
          });
          return;
        }
        const url = new URL(test.input, test.base || 'about:blank');
        strictEqual(url.href, test.href);
        strictEqual(url.protocol, test.protocol);
        strictEqual(url.username, test.username);
        strictEqual(url.password, test.password);
        strictEqual(url.host, test.host);
        strictEqual(url.hostname, test.hostname);
        strictEqual(url.port, test.port);
        strictEqual(url.pathname, test.pathname);
        strictEqual(url.search, test.search);
        if ('searchParams' in test) {
          strictEqual(url.searchParams.toString(), test.searchParams);
        }
        strictEqual(url.hash, test.hash);
        if (test.origin == null) {
          strictEqual(url.origin, 'null');
        } else {
          strictEqual(url.origin, test.origin);
        }
      });
    }

    // workerd: url-test.js:6898-7096 (wptTestURLSearchParamsConstructor):
    // ported subset; the DOMException block (6914-6920) and the instance-level
    // Symbol.iterator override block (7087-7094) are excluded — see README.md.
    {
      let params = new URLSearchParams();
      strictEqual(params + '', '');
      params = new URLSearchParams('');
      strictEqual(params + '', '');
      params = new URLSearchParams('a=b');
      strictEqual(params + '', 'a=b');
      params = new URLSearchParams(params);
      strictEqual(params + '', 'a=b');

      // workerd: url-test.js:6909-6912
      {
        const params = new URLSearchParams();
        strictEqual(params.toString(), '');
      }

      // workerd: url-test.js:6922-6925
      {
        const params = new URLSearchParams('');
        strictEqual(params.__proto__, URLSearchParams.prototype);
      }

      // workerd: url-test.js:6927-6930
      {
        const params = new URLSearchParams({});
        strictEqual(params + '', '');
      }

      // workerd: url-test.js:6932-6946
      {
        let params = new URLSearchParams('a=b');
        ok(params.has('a'));
        ok(!params.has('b'));
        params = new URLSearchParams('a=b&c');
        ok(params.has('a'));
        ok(params.has('c'));
        params = new URLSearchParams('&a&&& &&&&&a+b=& c&m%c3%b8%c3%b8');
        ok(params.has('a'));
        ok(params.has('a b'));
        ok(params.has(' '));
        ok(!params.has('c'));
        ok(params.has(' c'));
        ok(params.has('m\u00f8\u00f8'));
      }

      // workerd: url-test.js:6948-6960
      {
        const seed = new URLSearchParams('a=b&c=d');
        const params = new URLSearchParams(seed);
        strictEqual(params.get('a'), 'b');
        strictEqual(params.get('c'), 'd');
        ok(!params.has('d'));
        // The name-value pairs are copied when created; later updates
        // should not be observable.
        seed.append('e', 'f');
        ok(!params.has('e'));
        params.append('g', 'h');
        ok(!seed.has('g'));
      }

      // workerd: url-test.js:6962-6967
      {
        let params = new URLSearchParams('a=b+c');
        strictEqual(params.get('a'), 'b c');
        params = new URLSearchParams('a+b=c');
        strictEqual(params.get('a b'), 'c');
      }

      // workerd: url-test.js:6969-6978
      {
        const testValue = '+15555555555';
        const params = new URLSearchParams();
        params.set('query', testValue);
        const newParams = new URLSearchParams(params.toString());

        strictEqual(params.toString(), 'query=%2B15555555555');
        strictEqual(params.get('query'), testValue);
        strictEqual(newParams.get('query'), testValue);
      }

      // workerd: url-test.js:6980-6992
      {
        let params = new URLSearchParams('a=b c');
        strictEqual(params.get('a'), 'b c');
        params = new URLSearchParams('a b=c');
        strictEqual(params.get('a b'), 'c');
      }

      // workerd: url-test.js:6987-6992
      {
        let params = new URLSearchParams('a=b%20c');
        strictEqual(params.get('a'), 'b c');
        params = new URLSearchParams('a%20b=c');
        strictEqual(params.get('a b'), 'c');
      }

      // workerd: url-test.js:6994-7006
      {
        let params = new URLSearchParams('a=b\0c');
        strictEqual(params.get('a'), 'b\0c');
        params = new URLSearchParams('a\0b=c');
        strictEqual(params.get('a\0b'), 'c');
      }
      {
        let params = new URLSearchParams('a=b%00c');
        strictEqual(params.get('a'), 'b\0c');
        params = new URLSearchParams('a%00b=c');
        strictEqual(params.get('a\0b'), 'c');
      }

      // workerd: url-test.js:7008-7020
      {
        let params = new URLSearchParams('a=b\u2384');
        strictEqual(params.get('a'), 'b\u2384');
        params = new URLSearchParams('a\u2384b=c');
        strictEqual(params.get('a\u2384b'), 'c');
      }
      {
        let params = new URLSearchParams('a=b%e2%8e%84');
        strictEqual(params.get('a'), 'b\u2384');
        params = new URLSearchParams('a%e2%8e%84b=c');
        strictEqual(params.get('a\u2384b'), 'c');
      }

      // workerd: url-test.js:7022-7034
      {
        let params = new URLSearchParams('a=b\uD83D\uDCA9c');
        strictEqual(params.get('a'), 'b\uD83D\uDCA9c');
        params = new URLSearchParams('a\uD83D\uDCA9b=c');
        strictEqual(params.get('a\uD83D\uDCA9b'), 'c');
      }
      {
        let params = new URLSearchParams('a=b%f0%9f%92%a9c');
        strictEqual(params.get('a'), 'b\uD83D\uDCA9c');
        params = new URLSearchParams('a%f0%9f%92%a9b=c');
        strictEqual(params.get('a\uD83D\uDCA9b'), 'c');
      }

      // workerd: url-test.js:7036-7046
      {
        let params = new URLSearchParams([]);
        params = new URLSearchParams([
          ['a', 'b'],
          ['c', 'd'],
        ]);
        strictEqual(params.get('a'), 'b');
        strictEqual(params.get('c'), 'd');
        throws(() => new URLSearchParams([[1]]));
        throws(() => new URLSearchParams([[1, 2, 3]]));
      }

      // workerd: url-test.js:7048-7055
      {
        let params = new URLSearchParams('a=a/b~');
        strictEqual(params.toString(), 'a=a%2Fb%7E');

        let url = new URL('https://example.org?a=a/b~');
        strictEqual(url.search, '?a=a/b~');
        strictEqual(url.searchParams.toString(), 'a=a%2Fb%7E');
      }

      // workerd: url-test.js:7057-7085
      [
        { input: { '+': '%C2' }, output: [['+', '%C2']], name: 'object with +' },
        {
          input: { c: 'x', a: '?' },
          output: [
            ['c', 'x'],
            ['a', '?'],
          ],
          name: 'object with two keys',
        },
        {
          input: [
            ['c', 'x'],
            ['a', '?'],
          ],
          output: [
            ['c', 'x'],
            ['a', '?'],
          ],
          name: 'array with two keys',
        },
      ].forEach((val) => {
        let params = new URLSearchParams(val.input);
        let i = 0;
        for (let param of params) {
          deepStrictEqual(param, val.output[i]);
          i++;
        }
      });
    }

    // workerd: url-test.js:7098-7108 (urlSetSearch)
    {
      const url = new URL('http://example.com?foo=bar&baz=qux');
      url.search = '?quux=corge&grault=garply';
      const result = [];
      for (let param of url.searchParams) {
        result.push(param.join(':'));
      }
      deepStrictEqual(result, ['quux:corge', 'grault:garply']);
    }

    // workerd: url-test.js:7110-7120 (urlSegfaultRegression)
    {
      let url = new URL('http://example.org/?foo=bar&baz=qux');
      // Setting certain properties would once zero out an internal reference count, causing a
      // nondeterministic crash during URL/URLSearchParams destruction. It triggered an assertion
      // under debug mode when the `searchParams` property was accessed, which this test is designed
      // to express.
      url.protocol = 'http:';
      strictEqual(url.searchParams.toString(), 'foo=bar&baz=qux');
    }

    // workerd: url-test.js:7201-7216 (urlParseStatic)
    {
      const url = URL.parse('http://example.org');
      strictEqual(url.protocol, 'http:');
      strictEqual(url.host, 'example.org');

      const url2 = URL.parse('foo', 'http://example.org');
      strictEqual(url2.protocol, 'http:');
      strictEqual(url2.host, 'example.org');
      strictEqual(url2.pathname, '/foo');

      // Unlike `new URL(...)` the `URL.parse(...)` function does not throw on invalid URLs
      // (which is the key point)
      strictEqual(URL.parse('not valid'), null);
    }

    // workerd: url-test.js:7218-7224 (urlSearchParamsPipe)
    {
      const url = new URL('http://example.com/?a=b%7Cc');
      strictEqual(url.search, '?a=b%7Cc');
      strictEqual(url.searchParams.toString(), 'a=b%7Cc');
    }

    return 'workerd-url ok';
  },
};
