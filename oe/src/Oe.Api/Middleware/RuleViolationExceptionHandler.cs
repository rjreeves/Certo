using Microsoft.AspNetCore.Diagnostics;
using Microsoft.AspNetCore.Mvc;
using Oe.Domain.Exceptions;

namespace Oe.Api.Middleware;

/// <summary>
/// Maps RuleViolationException to RFC 9457 problem details with HTTP 422.
/// </summary>
internal sealed class RuleViolationExceptionHandler : IExceptionHandler
{
    public async ValueTask<bool> TryHandleAsync(
        HttpContext ctx,
        Exception   exception,
        CancellationToken ct)
    {
        if (exception is not RuleViolationException ex)
            return false;

        var problem = new ProblemDetails
        {
            Status   = StatusCodes.Status422UnprocessableEntity,
            Title    = "Business rule violation",
            Type     = "https://errors.oe.local/rule-violation",
            Instance = ctx.Request.Path,
            Extensions =
            {
                ["violations"] = ex.Violations.Select(v => new { v.Code, v.Message }),
            },
        };

        ctx.Response.StatusCode = StatusCodes.Status422UnprocessableEntity;
        await ctx.Response.WriteAsJsonAsync(problem, ct);
        return true;
    }
}
