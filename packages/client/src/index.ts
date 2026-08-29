import { RpcRequest, RpcResponse, AgentEvent, SessionMetadata, Message } from '@pi/core';

export interface ClientTransport {
  send(request: RpcRequest): Promise<RpcResponse>;
  onEvent(handler: (event: AgentEvent) => void): void;
}

export class PiClient {
  constructor(private transport: ClientTransport) {}

  async createSession(title: string, tags: string[] = []): Promise<SessionMetadata> {
    const res = await this.transport.send({
      id: `req-${Date.now()}`,
      method: 'session.create',
      params: { title, tags },
    });
    if (res.error) throw new Error(res.error.message);
    return res.result as SessionMetadata;
  }

  async runPrompt(sessionId: string, prompt: string): Promise<string> {
    const res = await this.transport.send({
      id: `req-${Date.now()}`,
      method: 'agent.run',
      params: { sessionId, prompt },
    });
    if (res.error) throw new Error(res.error.message);
    return (res.result as { response: string }).response;
  }

  async getMessages(sessionId: string): Promise<Message[]> {
    const res = await this.transport.send({
      id: `req-${Date.now()}`,
      method: 'session.getMessages',
      params: { sessionId },
    });
    if (res.error) throw new Error(res.error.message);
    return res.result as Message[];
  }
}
