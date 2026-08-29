import { Message, ToolCall } from '@pi/core';

export interface CompletionOptions {
  model: string;
  temperature?: number;
  maxTokens?: number;
  tools?: Array<{
    name: string;
    description: string;
    parameters: Record<string, unknown>;
  }>;
}

export interface CompletionResponse {
  content: string;
  toolCalls?: ToolCall[];
  finishReason: 'stop' | 'tool_calls' | 'length' | 'error';
}

export interface ILanguageModel {
  generate(messages: Message[], options: CompletionOptions): Promise<CompletionResponse>;
  stream(
    messages: Message[],
    options: CompletionOptions,
    onChunk: (chunk: string) => void
  ): Promise<CompletionResponse>;
}

export class MockLanguageModel implements ILanguageModel {
  async generate(messages: Message[], options: CompletionOptions): Promise<CompletionResponse> {
    const last = messages[messages.length - 1];
    return {
      content: `Echo: ${last?.content || ''}`,
      finishReason: 'stop',
    };
  }

  async stream(
    messages: Message[],
    options: CompletionOptions,
    onChunk: (chunk: string) => void
  ): Promise<CompletionResponse> {
    const last = messages[messages.length - 1];
    const text = `Echo: ${last?.content || ''}`;
    for (const char of text) {
      onChunk(char);
    }
    return {
      content: text,
      finishReason: 'stop',
    };
  }
}
