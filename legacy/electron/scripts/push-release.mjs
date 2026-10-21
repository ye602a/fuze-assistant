import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
const git = (...args) => execFileSync('git', args, { encoding: 'utf8' }).trim()
const { version } = JSON.parse(readFileSync('package.json', 'utf8'))
const tag = `v${version}`
if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error('Only stable Fuze versions can be published')
if (!/^https:\/\/github\.com\/ye602a\/fuze-assistant(?:\.git)?$/.test(git('remote', 'get-url', 'origin'))) throw new Error('origin must be the Fuze private repository')
if (git('status', '--porcelain')) throw new Error('Commit reviewed changes before publication')
if (git('cat-file', '-t', `refs/tags/${tag}`) !== 'tag' || git('rev-parse', `${tag}^{commit}`) !== git('rev-parse', 'HEAD')) throw new Error('Release needs an annotated tag at HEAD')
for (const ref of ['HEAD', `refs/tags/${tag}`]) execFileSync('git', ['push', 'origin', ref], { stdio: 'inherit' })
