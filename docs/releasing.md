# Releasing

oxsearch is released by hand to crates.io; there's no release workflow. The
steps, as for oxtel:

1. On a branch, set the workspace version in `Cargo.toml` and the versions in
   the crates' path dependencies, and move the changelog's `Unreleased`
   section under the new version, dated the day you'll publish. Add the
   release's specification branch to the README's version table.
2. Check that every crate packages and builds as it would from crates.io:

   ```bash
   cargo publish --workspace --dry-run
   ```

3. Open a pull request and merge it once CI is green.
4. Tag the merge commit with an annotated tag, `vX.Y.Z`, and push the tag.
5. From the tag, publish:

   ```bash
   cargo publish --workspace
   ```

   crates.io lets an account publish a few new crates at once, then about
   one every ten minutes, so a release with new crates stops with a 429
   partway. Publish the rest one at a time, in dependency order, as the
   limit allows (`cargo publish -p <crate>` from the tag).

6. Create the GitHub release from the tag, with the changelog's section for
   the version as its notes.
