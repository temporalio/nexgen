namespace Nexgen.AuthoredTypeShowcase
{
    // This part must compile with the generated operations under the authored
    // namespace and class name, rather than defining a separate empty class.
    public static partial class TypeShowcaseApi
    {
        public static string GetUserOperationName => nameof(GetUserAsync);
    }
}
