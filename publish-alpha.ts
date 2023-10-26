import * as child_process from 'child_process'

// publish packages as alpha and return the list of packages published
function publish() {
  try {
    const snapshot = child_process.execSync('yarn cs publish --tag alpha --no-git-tag')
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
    if (err instanceof Error) {
      process.stderr.write(err.message)
    } else {
      process.stderr.write((err as string).toString())
    }
  }
}

publish()
