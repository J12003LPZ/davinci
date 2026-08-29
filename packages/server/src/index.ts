import { RpcRequest, RpcResponse, AgentEvent } from '@pi/core';
import { ISessionStore } from '@pi/session-sqlite';
import { Agent } from '@pi/agent';

export class PiServer {
  constructor(
    private sessionStore: ISessionStore,
    private agent: Agent
  ) {}

  async handleRpc(request: RpcRequest, onEvent?: (event: AgentEvent) => void): Promise<RpcResponse> {
    try {
      switch (request.method) {
        case 'session.create': {
          const { title, tags } = request.params as { title: string; tags?: string[] };
          const id = `sess-${Date.now()}`;
          const session = await this.sessionStore.createSession(id, title, tags);
          return { id: request.id, result: session };
        }
        case 'session.list': {
          const sessions = await this.sessionStore.listSessions();
          return { id: request.id, result: sessions };
        }
        case 'session.getMessages': {
          const { sessionId } = request.params as { sessionId: string };
          const messages = await this.sessionStore.getMessages(sessionId);
          return { id: request.id, result: messages };
        }
        case 'agent.run': {
          const { sessionId, prompt } = request.params as { sessionId: string; prompt: string };
          const response = await this.agent.run(sessionId, prompt, onEvent);
          return { id: request.id, result: { response } };
        }
        default:
          return {
            id: request.id,
            error: { code: -32601, message: `Method not found: ${request.method}` },
          };
      }
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : String(err);
      return {
        id: request.id,
        error: { code: -32000, message },
      };
    }
  }
}
