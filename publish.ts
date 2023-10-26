import * as child_process from 'child_process'

const FAKE_OUTPUT = `
🦋  info npm info @celo/phone-number-privacy-common
🦋  info npm info @celo/encrypted-backup
🦋  info npm info @celo/identity
🦋  info @celo/phone-number-privacy-common is being published because our local version (3.0.4-alpha-964a99345) has not been published on npm
🦋  warn @celo/encrypted-backup is not being published because version 5.0.4 is already published on npm
🦋  warn @celo/identity is not being published because version 5.0.4 is already published on npm
🦋  info Publishing "@celo/phone-number-privacy-common" at "3.0.4-alpha-964a99345"
🦋  info Publishing "@celo/identity" at "3.0.4-alpha-964a99345"
`
// publish packages as alpha and return the list of packages published
function publish() {
  try {
    const snapshot = child_process.execSync('yarn cs publish --tag alpha --no-git-tag')
    // const snapshot = child_process.execSync(`echo "${FAKE_OUTPUT}"`)
    const arrayOfLines = snapshot
      .toString()
      .split('\n')
      .map((line: string) => {
        const matches = line.match(/info Publishing @celo.*$/)
        return matches
      })

    const pkgs = arrayOfLines
      .filter((line) => !!line)
      .map((line) => {
        if (!line) return
        return line[0].replace('info Publishing ', '').replace(' at ', '@')
      })
    const result = pkgs.length ? JSON.stringify(pkgs) : 'no-op'
    process.stdout.write(result)
  } catch (err) {
    process.stderr.write(err)
  }
}

publish()
