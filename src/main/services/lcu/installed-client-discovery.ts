import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import path from 'node:path'
import https from 'node:https'
import { inspectLeagueInstallDirectory } from '../../modules/lol-path.ts'
import { discoverLcuAuthFromManualDirectory } from './manual-directory-auth.ts'
import { getClientDiscoverySnapshot } from './process-auth-discovery.ts'
import type { TokenLoadResult } from './types.ts'

const exec = promisify(execFile)
// Fixed read-only metadata sources. No commands, shortcut arguments or registry values are executed.
export const INSTALL_DISCOVERY_SCRIPT = String.raw`
$ErrorActionPreference='SilentlyContinue'
[Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false)
$result = [System.Collections.Generic.List[string]]::new()
foreach($key in @('HKCU:\Software\Tencent\LOL','HKLM:\SOFTWARE\Tencent\LOL','HKLM:\SOFTWARE\WOW6432Node\Tencent\LOL')) {
  $v=Get-ItemProperty -LiteralPath $key
  foreach($name in @('InstallPath','InstallLocation','GamePath','Path')) { if($v.$name -is [string]){$result.Add($v.$name)} }
}
foreach($root in @('HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall','HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall','HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall')) {
  Get-ChildItem -LiteralPath $root | Select-Object -First 512 | ForEach-Object {
    $v=Get-ItemProperty -LiteralPath $_.PSPath
    if($v.DisplayName -match 'League of Legends|英雄联盟|^LOL$') {
      if($v.InstallLocation){$result.Add([string]$v.InstallLocation)}
      if($v.DisplayIcon){$result.Add(([string]$v.DisplayIcon -replace ',\s*-?\d+$','').Trim('"'))}
    }
  }
}
$shell=New-Object -ComObject WScript.Shell
foreach($root in @([Environment]::GetFolderPath('Desktop'),[Environment]::GetFolderPath('CommonDesktopDirectory'),[Environment]::GetFolderPath('StartMenu'),[Environment]::GetFolderPath('CommonStartMenu'))) {
  if(!$root){continue}
  Get-ChildItem -LiteralPath $root -Filter '*.lnk' -File -Recurse -Depth 2 | Select-Object -First 256 | Where-Object Name -Match 'League|英雄联盟|^LOL' | ForEach-Object {
    $target=$shell.CreateShortcut($_.FullName).TargetPath
    if($target){$result.Add([string]$target)}
  }
}
@($result | Select-Object -Unique -First 32) | ConvertTo-Json -Compress
`

export function normalizeInstallCandidates(value: unknown): string[] {
  const candidates: string[] = []
  for (const raw of (Array.isArray(value) ? value : [value]).slice(0, 32)) {
    if (typeof raw !== 'string') continue
    let entry = raw.trim().replace(/^"|"$/g, '')
    // Local drive paths only: never touch UNC shares, URLs or relative executable names.
    if (!/^[a-z]:[\\/]/i.test(entry) || entry.length > 1024 || [...entry].some(c => c.charCodeAt(0) < 32)) continue
    entry = path.win32.normalize(entry)
    if (/\.exe$/i.test(entry)) entry = path.win32.dirname(entry)
    if (/^(TCLS|Game)$/i.test(path.win32.basename(entry))) entry = path.win32.dirname(entry)
    if (!candidates.some(item => item.toLowerCase() === entry.toLowerCase())) candidates.push(entry)
  }
  return candidates
}

export async function collectInstalledClientDirectories(): Promise<string[]> {
  if (process.platform !== 'win32') return []
  try {
    const { stdout } = await exec('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', INSTALL_DISCOVERY_SCRIPT],
      { timeout: 5000, maxBuffer: 65536, windowsHide: true })
    const candidates = normalizeInstallCandidates(JSON.parse(stdout.trim() || '[]'))
    const valid: string[] = []
    for (const candidate of candidates) {
      if ((await inspectLeagueInstallDirectory(candidate)).valid) valid.push(candidate)
      if (valid.length === 6) break
    }
    return valid
  } catch { return [] }
}

// Candidate discovery is not connection success. Check an explicit read-only endpoint before caching.
export function verifyDirectoryAuth(auth: TokenLoadResult): Promise<boolean> {
  const [token, port] = auth
  if (!token || !port || !/^\d{1,5}$/.test(port) || +port < 1 || +port > 65535) return Promise.resolve(false)
  return new Promise(resolve => {
    let done = false, body = ''
    let timer: ReturnType<typeof setTimeout> | undefined
    const finish = (ok: boolean) => { if (!done) { done = true; clearTimeout(timer); resolve(ok) } }
    const request = https.get({ hostname: '127.0.0.1', port: +port, path: '/lol-gameflow/v1/gameflow-phase',
      auth: `riot:${token}`, rejectUnauthorized: false, agent: false }, response => {
      if (response.statusCode !== 200) { response.destroy(); finish(false); return }
      response.on('data', chunk => { body += chunk.toString('utf8'); if (body.length > 256) { response.destroy(); finish(false) } })
      response.on('end', () => { try { finish(['None', 'Lobby', 'Matchmaking', 'ReadyCheck', 'ChampSelect', 'GameStart', 'InProgress', 'Reconnect', 'WaitingForStats', 'PreEndOfGame', 'EndOfGame', 'TerminatedInError'].includes(JSON.parse(body))) } catch { finish(false) } })
      response.on('error', () => finish(false)); response.on('aborted', () => finish(false))
    })
    timer = setTimeout(() => { request.destroy(); finish(false) }, 800)
    timer.unref(); request.on('error', () => finish(false))
  })
}

export class InstalledClientDiscovery {
  private directories: string[] = []
  private nextScan = 0
  private nextAttempt = 0
  private pending: Promise<TokenLoadResult> | null = null
  verifiedDirectory: string | null = null
  constructor(private collect = collectInstalledClientDirectories, private read = discoverLcuAuthFromManualDirectory,
    private verify = verifyDirectoryAuth, private now = Date.now) {}
  discover(): Promise<TokenLoadResult> {
    if (this.pending) return this.pending
    if (this.now() < this.nextAttempt) return Promise.resolve([null, null, null])
    this.nextAttempt = this.now() + 30000
    this.pending = this.run().finally(() => { this.pending = null })
    return this.pending
  }
  private async run(): Promise<TokenLoadResult> {
    if (this.now() >= this.nextScan) { this.nextScan = this.now() + 300000; this.directories = (await this.collect()).slice(0, 6) }
    const previous = this.verifiedDirectory
    this.verifiedDirectory = null
    for (const directory of [...new Set([previous, ...this.directories].filter((v): v is string => !!v))]) {
      try {
        const auth = await this.read(directory)
        if (auth[0] && auth[1] && await this.verify(auth)) { this.verifiedDirectory = directory; return auth }
      } catch { /* Inaccessible/stale candidates are not successful discoveries. */ }
    }
    return [null, null, null]
  }
}
const installed = new InstalledClientDiscovery()
export const getInstalledClientDirectory = () => installed.verifiedDirectory
export const discoverLcuAuthFromInstalledDirectory = () => getClientDiscoverySnapshot().processCount > 0
  ? installed.discover() : Promise.resolve<TokenLoadResult>([null, null, null])
