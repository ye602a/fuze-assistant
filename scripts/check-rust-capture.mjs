// Only generated GPU pixels and our hidden prototype window. Never enumerate/capture a game.
import { writeFile } from 'node:fs/promises'
import path from 'node:path'
import { run, output } from './rust-prototype-tools.mjs'
const adapters = run('fuze-capture', ['--adapters'])
const hardware = adapters.exit_ok && adapters.adapters?.some(a => a.index === 0 && !a.software)
const readback = hardware ? run('fuze-capture', ['--synthetic-readback']) : { skipped: 'no_hardware_adapter' }
const previous = process.env.FUZE_RUST_WGC_TEST
let window
try {
  process.env.FUZE_RUST_WGC_TEST = '1'
  window = run('fuze-native-overlay', ['--smoke', path.join(output, 'native-overlay.png'), path.join(output, 'snapshot.json')])
} finally {
  if (previous === undefined) delete process.env.FUZE_RUST_WGC_TEST
  else process.env.FUZE_RUST_WGC_TEST = previous
}
const report = { adapters, readback, window, gameCaptureValidated: false, source: 'generated_texture_and_owned_hidden_window' }
await writeFile(path.join(output, 'capture-validation.json'), JSON.stringify(report, null, 2))
console.log(JSON.stringify(report))
// Hidden HWND can be rejected by WGC; it is NOT reported as a successful capture.
if (!adapters.exit_ok || (hardware && (!readback.exit_ok || !readback.passed)) ||
    !window.exit_ok || !window.hidden || !window.foreground_unchanged || !window.wgc) process.exitCode = 1
