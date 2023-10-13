module.exports = {
  async constraints({ Yarn }) {
    for (const workspace of Yarn.workspaces()) {
      workspace.set('engines.node', `18`)
    }
  },
}
