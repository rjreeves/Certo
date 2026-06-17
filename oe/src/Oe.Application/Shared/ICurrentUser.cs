using Oe.Domain.Enums;

namespace Oe.Application.Shared;

public interface ICurrentUser
{
    Guid     Id   { get; }
    UserRole Role { get; }
}
