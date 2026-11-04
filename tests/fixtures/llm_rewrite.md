Git v2.54 Release Notes

UI, Workflows & Features

* The "git add -p" command and its related tools now seamlessly display the current status of the hunk being shown, providing users with valuable context.

* A powerful new experimental command, "git history", has been introduced to streamline history rewriting workflows.

* Notably, "git replay" can now intelligently drop commits that become empty — while preserving the ones that were empty in the original.

* The help text and documentation for the "--expire" option of "git worktree [list|prune]" have been meticulously enhanced to foster a clearer understanding.

* When "git show-index" is run outside a repository, it silently defaults to SHA-1. It's worth noting that the tool now proactively warns users when this happens.

* Furthermore, "git merge-file" can be run outside a repository; however, it previously ignored all configuration. The command now leverages available configuration files to ensure a seamless and robust experience.

* "git rev-list" and related commands now offer a "--maximal-only" option, which plays a crucial role in showing only the commits that are not reachable by other commits.

* Additionally, the alias configuration syntax has been extended to support aliases using characters outside ASCII alphanumerics — empowering users to craft more expressive workflows.

* "gitweb" now delivers a comprehensive, mobile-friendly experience, underscoring Git's commitment to accessibility in today's ever-evolving landscape.

* "git maintenance" now utilizes the "geometric" strategy by default, ultimately delivering more efficient repository upkeep.
