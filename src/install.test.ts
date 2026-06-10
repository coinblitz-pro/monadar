import assert from 'node:assert/strict'
import test from 'node:test'
import { buildLaunchdPlist, buildSystemdUnit, quoteSystemdArg } from './install'

test('quoteSystemdArg quotes whitespace and escapes systemd specifiers', () => {
  assert.equal(quoteSystemdArg('/tmp/a b/monadar.js'), '"/tmp/a b/monadar.js"')
  assert.equal(quoteSystemdArg('/tmp/%h/$HOME'), '"/tmp/%%h/$$HOME"')
})

test('buildSystemdUnit quotes argv in ExecStart', () => {
  const unit = buildSystemdUnit([ '/usr/bin/node', '/tmp/a b/monadar.js', 'daemon' ])
  assert.match(unit, /^ExecStart=\/usr\/bin\/node "\/tmp\/a b\/monadar\.js" daemon$/m)
})

test('buildLaunchdPlist XML-escapes argv values', () => {
  const plist = buildLaunchdPlist([ '/usr/bin/node', '/tmp/a & b/<monadar>.js' ])
  assert.match(plist, /<string>\/tmp\/a &amp; b\/&lt;monadar&gt;\.js<\/string>/)
})
