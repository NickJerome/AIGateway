// Ported assertions from workerd's src/workerd/api/tests/encoding-test.js
// (cloudflare/workerd, Apache-2.0). Only the UTF-8 parts are ported: all
// non-UTF-8 encodings (big5/utf-16/gbk/iso2022jp/x-user-defined and the
// windows-1252/ascii/shift_jis sections of decodeStreamingTest) and every
// *Stream test are excluded — see README.md in this directory.
// Every section carries a `// workerd: <file>:<start>-<end>` provenance mark.
//
// The host harness prepends node-assert-shim.mjs, which provides
// strictEqual / deepStrictEqual / ok / throws / match / fail.

// workerd: encoding-test.js:9-19 (decodeStreaming helper, verbatim)
function decodeStreaming(decoder, input) {
  // Test truncation behavior while streaming by feeding the decoder a single byte at a time.
  // Note we don't try-catch here because we don't expect this ever to fail, because we're
  // streaming text.
  let x = '';
  for (let i = 0; i < input.length; ++i) {
    x += decoder.decode(input.slice(i, i + 1), { stream: true });
  }
  x += decoder.decode();
  return x;
}

// workerd: encoding-test.js:42 (utf8Labels, verbatim; the windows1252Labels
// list at lines 22-40 is excluded as non-UTF-8)
const utf8Labels = ['unicode-1-1-utf-8', 'utf-8', 'utf8'];

export default {
  async fetch() {
    // workerd: encoding-test.js:44-178 (decodeStreamingTest, UTF-8 subset;
    // windows-1252 labels L48-53, the ascii section L114-123 and the
    // shift_jis section L161-176 are excluded as non-UTF-8)
    {
      // workerd: encoding-test.js:54-59
      for (const label of utf8Labels) {
        ok(
          new TextDecoder(`${label}`).encoding === 'utf-8',
          `TextDecoder constructed with '${label}' label to have 'utf-8' encoding.`
        );
      }

      // workerd: encoding-test.js:61-70
      const decoder = new TextDecoder();
      const fatalDecoder = new TextDecoder('utf-8', { fatal: true });
      const fatalIgnoreBomDecoder = new TextDecoder('utf-8', {
        fatal: true,
        ignoreBOM: true,
      });

      ok(decoder.encoding === 'utf-8', "default encoding property to be 'utf-8'");
      ok(decoder.fatal === false, 'default fatal property to be false');
      ok(decoder.ignoreBOM === false, 'default ignoreBOM property to be false');

      // workerd: encoding-test.js:72-82
      const fooCat = new Uint8Array([102, 111, 111, 32, 240, 159, 152, 186]);

      ok(decoder.decode().length === 0, 'decoded undefined array length to be 0');
      ok(decoder.decode(fooCat) === 'foo \u{1F63A}', 'foo-cat from Uint8Buffer to be foo \u{1F63A}');
      ok(
        decoder.decode(fooCat.buffer) === 'foo \u{1F63A}',
        'foo-cat from ArrayBuffer to be foo \u{1F63A}'
      );

      // workerd: encoding-test.js:84-112
      const twoByteCodePoint = new Uint8Array([0xc2, 0xa2]); // cent sign
      const threeByteCodePoint = new Uint8Array([0xe2, 0x82, 0xac]); // euro sign
      const fourByteCodePoint = new Uint8Array([240, 159, 152, 186]); // cat emoji

      [twoByteCodePoint, threeByteCodePoint, fourByteCodePoint].forEach((input) => {
        // For each input sequence of code units, try decoding each subsequence of code units except the
        // full code point itself.
        for (let i = 1; i < input.length; ++i) {
          const head = input.slice(0, input.length - i);
          const tail = input.slice(input.length - i);

          ok(
            decoder.decode(head) === '\uFFFD',
            'code point fragment (head) to be replaced with replacement character'
          );
          ok(
            decoder.decode(tail) === '\uFFFD'.repeat(tail.length),
            'code point fragment (tail) to be replaced with replacement character'
          );

          // Exception to be thrown decoding code point fragment (tail) in fatal mode
          throws(() => fatalDecoder.decode(head));
          throws(() => fatalDecoder.decode(tail));
        }
      });

      // workerd: encoding-test.js:125-138 (Test streaming)
      ok(
        decodeStreaming(fatalDecoder, twoByteCodePoint) === '\u00A2',
        '2-byte code point (cent sign) to be decoded correctly'
      );
      ok(
        decodeStreaming(fatalDecoder, threeByteCodePoint) === '\u20AC',
        '3-byte code point (euro sign) to be decoded correctly'
      );
      ok(
        decodeStreaming(fatalDecoder, fourByteCodePoint) === '\u{1F63A}',
        '4-byte code point (cat emoji) to be decoded correctly'
      );

      // workerd: encoding-test.js:140-159 (BOM handling)
      const bom = new Uint8Array([0xef, 0xbb, 0xbf]);
      const bomBom = new Uint8Array([0xef, 0xbb, 0xbf, 0xef, 0xbb, 0xbf]);

      ok(
        decodeStreaming(fatalDecoder, bom) === '',
        'BOM to be stripped by TextDecoder without ignoreBOM set'
      );
      ok(
        decodeStreaming(fatalDecoder, bomBom) === '\ufeff',
        'first BOM to be stripped by TextDecoder without ignoreBOM set'
      );

      ok(
        decodeStreaming(fatalIgnoreBomDecoder, bom) === '\ufeff',
        'BOM not to be stripped by TextDecoder with ignoreBOM set'
      );
      ok(
        decodeStreaming(fatalIgnoreBomDecoder, bomBom) === '\ufeff\ufeff',
        'first BOM not to be stripped by TextDecoder with ignoreBOM set'
      );
    }

    // workerd: encoding-test.js:180-190 (textEncoderTest, verbatim)
    {
      const encoder = new TextEncoder();
      strictEqual(encoder.encoding, 'utf-8');
      strictEqual(encoder.encode().length, 0);
      deepStrictEqual(
        encoder.encode('foo \u{1F63A}'),
        new Uint8Array([102, 111, 111, 32, 240, 159, 152, 186])
      );
    }

    // workerd: encoding-test.js:192-385 (encodeWptTest): only w3cTestEncode
    // (L197-238) is ported; w3cTestEncodeInto (L240-383) is excluded because
    // TextEncoder.encodeInto is not implemented in this runtime (README.md).
    {
      // workerd: encoding-test.js:198-229 (WPT surrogate test table)
      const bad = [
        {
          input: '\uD800',
          expected: '\uFFFD',
          name: 'lone surrogate lead',
        },
        {
          input: '\uDC00',
          expected: '\uFFFD',
          name: 'lone surrogate trail',
        },
        {
          input: '\uD800\u0000',
          expected: '\uFFFD\u0000',
          name: 'unmatched surrogate lead',
        },
        {
          input: '\uDC00\u0000',
          expected: '\uFFFD\u0000',
          name: 'unmatched surrogate trail',
        },
        {
          input: '\uDC00\uD800',
          expected: '\uFFFD\uFFFD',
          name: 'swapped surrogate pair',
        },
        {
          input: '\uD834\uDD1E',
          expected: '\uD834\uDD1E',
          name: 'properly encoded MUSICAL SYMBOL G CLEF (U+1D11E)',
        },
      ];

      // workerd: encoding-test.js:231-237
      bad.forEach(function (t) {
        const encoded = new TextEncoder().encode(t.input);
        const decoded = new TextDecoder().decode(encoded);
        strictEqual(decoded, t.expected);
      });

      strictEqual(new TextEncoder().encode().length, 0);
    }

    return 'workerd-encoding ok';
  },
};
