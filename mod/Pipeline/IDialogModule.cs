namespace ErenshorLLMDialog.Pipeline
{
    public interface IInputModule
    {
        void Process(DialogContext ctx, TypeText typeText);
    }

    public interface ISampleModule
    {
        void Sample(DialogContext ctx);
    }

    public interface ITransformModule
    {
        /// <returns>true if this module handled the message</returns>
        bool Transform(DialogContext ctx);
    }

    public interface IOutputModule
    {
        void Output(DialogContext ctx);
    }
}
