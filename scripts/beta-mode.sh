#!/usr/bin/env bash

# Beta Workflow steps
# 0. Enter any random branch name
echo "Enter a branch name"
read branch_name
# 1. create a prerelease/random branch
git branch prerelease/$branch_name
# 2. check it out
git checkout prerelease/$branch_name
# 3. enter pre mode (beta)
yarn cs pre enter beta
# 4. commit
git add .changeset/pre.json
git commit -am "enter beta mode"
# 5. push
git push origin prerelease/$branch_name
# 6. githhub action will automatically trigger and open a Version packages (beta) PR against the prerelease/random branch
echo "Commit to this prerelease/$branch_name and push up will trigger the github action to open a Version packages (beta) PR"
# 7. merge that PR to publish or Push up more commits to update
#    a. if you do merge/publish you will need to add more changesets to initiate a new beta being published
# 8. repeat 7 if wanted
# 9. when ready to exit pre mode. `yarn beta-exit`
echo "when complete with beta mode, run yarn beta-exit"
# 11. open PR for prerelease into main
