# \PersonHandlerApi

All URIs are relative to *http://127.0.0.1:8070*

Method | HTTP request | Description
------------- | ------------- | -------------
[**create_person_handler**](PersonHandlerApi.md#create_person_handler) | **POST** /api/v1/persons | 
[**delete_person_handler**](PersonHandlerApi.md#delete_person_handler) | **DELETE** /api/v1/persons/{id} | 
[**edit_person_handler**](PersonHandlerApi.md#edit_person_handler) | **PATCH** /api/v1/persons/{id} | 
[**get_person_handler**](PersonHandlerApi.md#get_person_handler) | **GET** /api/v1/persons/{id} | 
[**person_list_handler**](PersonHandlerApi.md#person_list_handler) | **GET** /api/v1/persons | 
[**person_list_search_handler**](PersonHandlerApi.md#person_list_search_handler) | **GET** /api/v1/persons/search | 



## create_person_handler

> models::PersonModel create_person_handler(create_person_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**create_person_schema** | [**CreatePersonSchema**](CreatePersonSchema.md) | Create person | [required] |

### Return type

[**models::PersonModel**](PersonModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## delete_person_handler

> delete_person_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | Person id for deleting person | [required] |

### Return type

 (empty response body)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## edit_person_handler

> models::PersonModel edit_person_handler(id, update_person_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | Person id for patching person | [required] |
**update_person_schema** | [**UpdatePersonSchema**](UpdatePersonSchema.md) | Update person | [required] |

### Return type

[**models::PersonModel**](PersonModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## get_person_handler

> models::PersonModel get_person_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | Person id for fetching person | [required] |

### Return type

[**models::PersonModel**](PersonModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## person_list_handler

> models::PagedResponsePersonModel person_list_handler(page, limit, order_by, desc, name, pid)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**page** | Option<**i32**> | page number |  |
**limit** | Option<**i32**> | page size |  |
**order_by** | Option<**String**> | list order by |  |
**desc** | Option<**bool**> | should use descending instead |  |
**name** | Option<**String**> | person's name |  |
**pid** | Option<**String**> | person's pid |  |

### Return type

[**models::PagedResponsePersonModel**](PagedResponse_PersonModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## person_list_search_handler

> models::PagedResponsePersonModel person_list_search_handler(page, limit, order_by, desc, query)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**page** | Option<**i32**> | page number |  |
**limit** | Option<**i32**> | page size |  |
**order_by** | Option<**String**> | list order by |  |
**desc** | Option<**bool**> | should use descending instead |  |
**query** | Option<**String**> | query by person's name, surname and pid |  |

### Return type

[**models::PagedResponsePersonModel**](PagedResponse_PersonModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

