using System;

namespace ErenshorLLMDialog.Pipeline.Transform
{
    public class HelloWorldTransform : ITransformModule
    {
        public bool Transform(DialogContext ctx)
        {
            if (ctx.Channel != ChatChannel.Say)
                return false;

            if (ctx.TargetSimPlayer == null)
                return false;

            if (ctx.PlayerMessage.IndexOf("hello world", StringComparison.OrdinalIgnoreCase) < 0)
                return false;

            ctx.TransformedResponse = "I am not your world!";
            ctx.Handled = true;
            ctx.PipelineLog.Add("[HelloWorldTransform] Matched 'hello world', set response");
            return true;
        }
    }
}
