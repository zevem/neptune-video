# Server, transports and safety

Status: ready-for-human
Progress: resolved
Type: task

## Scope

`neptune-video-mcp` on rmcp 3.5.1 over stdio and loopback Streamable HTTP; roots,
bearer token, Host and Origin checks, body cap, connection file, stdout
isolation, signals; the authority thread and its bounded queue.

## Result

`src/mcp/{mod,http,server,workspace,paths,errors,time}.rs`,
`src/bin/neptune-video-mcp.rs`. Tested by `tests/mcp.rs`
(`http_answers_only_its_token_on_the_loopback_and_works_for_a_client`,
`files_outside_the_roots_are_refused_and_nothing_is_replaced_unasked`) and
the `mcp::paths` unit tests; curl checks gave 401, 401, 403, 403 and 413 for
a missing token, a wrong token, a foreign Host, a browser Origin and a 5 MB
body.

## Comments

2026-10-14: SIGTERM, SIGHUP and SIGINT now release the project as closing
stdin does; real clients stop their servers with a signal and left
recovery copies behind before the fix.

2026-10-14: Review fixes: dangling links refused at destinations; pictures
and caption files written through new hidden files renamed into place;
reference formats refused by probes; links in walked folders not read;
fonts outside the roots stop renders; attaching fails closed. Tested by
`links_and_lists_inside_the_roots_cannot_reach_outside_them`.
