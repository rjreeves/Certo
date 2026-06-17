using Oe.Api.Endpoints;
using Oe.Api.Middleware;
using Oe.Application.Invoices;
using Oe.Application.Orders;
using Oe.Application.Payments;
using Oe.Application.Shared;
using Oe.Infrastructure.Database;
using Oe.Infrastructure.Repositories;

var builder = WebApplication.CreateBuilder(args);

// ------------------------------------------------------------------ //
// Services
// ------------------------------------------------------------------ //

var connectionString = builder.Configuration.GetConnectionString("Oe")
    ?? throw new InvalidOperationException("Connection string 'Oe' not found.");

builder.Services.AddSingleton(new DbConnectionFactory(connectionString));

builder.Services.AddHttpContextAccessor();
builder.Services.AddScoped<ICurrentUser, HttpCurrentUser>();

builder.Services.AddScoped<ISubmitOrderHandler,  OrderRepository>();
builder.Services.AddScoped<IOrderLineHandler,     OrderRepository>();
builder.Services.AddScoped<IVoidInvoiceHandler,   InvoiceRepository>();
builder.Services.AddScoped<ICreatePaymentHandler, PaymentRepository>();

builder.Services.AddExceptionHandler<RuleViolationExceptionHandler>();
builder.Services.AddProblemDetails();

builder.Services.AddAuthentication().AddJwtBearer();
builder.Services.AddAuthorization();

// ------------------------------------------------------------------ //
// Pipeline
// ------------------------------------------------------------------ //

var app = builder.Build();

app.UseExceptionHandler();
app.UseAuthentication();
app.UseAuthorization();

app.MapOrderEndpoints();
app.MapInvoiceEndpoints();
app.MapPaymentEndpoints();

app.Run();
