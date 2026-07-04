---
name: verify-before-done
description: Exit gate for coding turns — never end an implementation turn without written files and passing verification.
---
Exit gate — before you end any turn that asked you to implement or modify code:

1. Files: did you save the changes with write_file or edit_file this turn? If
   not, write them now — ending an implementation turn with only prose is a
   failure.
2. Verify: did you run the verification commands the task asked for (tests,
   typecheck)? If not, run them now with execute_command.
3. Green: if verification failed, fix the code and re-run it.

Only after all three: give your short final reply.
