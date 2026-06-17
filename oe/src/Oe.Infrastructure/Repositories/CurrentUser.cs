using System.Security.Claims;
using Microsoft.AspNetCore.Http;
using Oe.Application.Shared;
using Oe.Domain.Enums;

namespace Oe.Infrastructure.Repositories;

public sealed class HttpCurrentUser(IHttpContextAccessor accessor) : ICurrentUser
{
    private ClaimsPrincipal User => accessor.HttpContext!.User;

    public Guid Id =>
        Guid.Parse(User.FindFirst(ClaimTypes.NameIdentifier)?.Value
            ?? throw new InvalidOperationException("User ID claim missing."));

    public UserRole Role =>
        Enum.Parse<UserRole>(
            User.FindFirst(ClaimTypes.Role)?.Value
                ?? throw new InvalidOperationException("Role claim missing."),
            ignoreCase: true);
}
