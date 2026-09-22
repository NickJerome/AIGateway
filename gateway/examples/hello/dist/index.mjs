export default {
  async fetch(request, env) {
    if (request.method === 'PUT') {
      await env.CONFIG.put('message', await request.text());
      return new Response(null, { status: 204 });
    }
    const message = await env.CONFIG.get('message');
    return Response.json({ message: message ?? env.GREETING });
  }
};
