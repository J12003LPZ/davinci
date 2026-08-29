import { Message, AgentEvent, ToolCall } from '@pi/core';
import { ILanguageModel, CompletionOptions } from '@pi/ai';
import { ISessionStore } from '@pi/session-sqlite';

export interface ITool {
  name: string;
  description: string;
  parameters: Record<string, unknown>;
  execute(args: Record<string, unknown>): Promise<string>;
}

export class Agent {
  private tools: Map<string, ITool> = new Map();

  constructor(
    private sessionStore: ISessionStore,
    private model: ILanguageModel,
    private agentId: string = 'agent-default'
  ) {}

  registerTool(tool: ITool): void {
    this.tools.set(tool.name, tool);
  }

  async run(sessionId: string, prompt: string, onEvent?: (event: AgentEvent) => void): Promise<string> {
    const hasLease = await this.sessionStore.acquireWriterLease(sessionId, this.agentId, 30000);
    if (!hasLease) {
      throw new Error(`Failed to acquire writer lease for session ${sessionId}`);
    }

    try {
      const now = Date.now();
      onEvent?.({ type: 'started', sessionId, timestamp: now });

      const userMsg: Message = {
        id: `msg-${Date.now()}-user`,
        sessionId,
        role: 'user',
        content: prompt,
        timestamp: now,
      };
      await this.sessionStore.appendMessage(userMsg);

      const history = await this.sessionStore.getMessages(sessionId);
      const toolDefs = Array.from(this.tools.values()).map(t => ({
        name: t.name,
        description: t.description,
        parameters: t.parameters,
      }));

      const options: CompletionOptions = {
        model: 'default',
        tools: toolDefs.length > 0 ? toolDefs : undefined,
      };

      let fullContent = '';
      const response = await this.model.stream(history, options, chunk => {
        fullContent += chunk;
        onEvent?.({ type: 'message_chunk', sessionId, chunk });
      });

      const assistantMsg: Message = {
        id: `msg-${Date.now()}-assistant`,
        sessionId,
        role: 'assistant',
        content: fullContent || response.content,
        timestamp: Date.now(),
      };
      await this.sessionStore.appendMessage(assistantMsg);

      onEvent?.({ type: 'completed', sessionId, timestamp: Date.now() });
      return assistantMsg.content;
    } finally {
      await this.sessionStore.releaseWriterLease(sessionId, this.agentId);
    }
  }
}
