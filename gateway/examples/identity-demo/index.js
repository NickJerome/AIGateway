export default {
  fetch(request, env) {
    if (new URL(request.url).pathname === "/identity/health") {
      return Response.json({ ok: true });
    }
    // Authentication has already completed in Core. Business configuration can
    // use these IDs, without receiving or rechecking the raw credential.
    return Response.json({ identity: env.IDENTITY });
  },
};
