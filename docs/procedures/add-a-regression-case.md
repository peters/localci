# Add a regression case

Use this procedure when localci does a different thing from GitHub, or when
`localci check` shows an error.

**CAUTION:** DO NOT COPY CONTENT FROM A PRIVATE WORKFLOW. The localci
repository can become public. Write a new workflow that shows only the problem.

1. Find the next free number in `tests/cases/`.
2. Make a folder with the name `NNNN-short-name`.
3. Write the smallest workflow that shows the problem in `workflow.yml`.
4. Write `case.json`. Use an existing case as an example.
5. Run the case:

   ```sh
   cargo test --test cases -- --nocapture
   ```

   Result: The new case fails, and the output shows the difference.

6. Fix the code.
7. Run all tests:

   ```sh
   cargo test && cargo clippy --all-targets -- -D warnings
   ```

   Result: All tests pass.

8. If you have private workflows, run the corpus check:

   ```sh
   LOCALCI_CORPUS=/path/to/repositories cargo test --test corpus
   ```

   Result: The test passes.

9. Add a row to `docs/compatibility.md`.
10. If the fix sets new behavior, write an ADR. The template is in
    `docs/adr/README.md`.
